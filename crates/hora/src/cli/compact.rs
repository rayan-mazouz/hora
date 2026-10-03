//! `hora compact`: give a database's free pages back to the filesystem.

use anyhow::Context as _;
use hora_core::db::{self, LockError, SizeReport};
use hora_core::fmt::bytes;

use super::{CliError, open_database, usage};

/// Room the copy needs on top of its expected size (the estimate is rough,
/// and the retention pass writes a WAL first).
const DISK_MARGIN_PERCENT: u64 = 10;

/// `hora compact [--dry-run] [--purge-removed] [--keep]`.
///
/// Applies the retention now, then rewrites the file with `VACUUM INTO` a
/// sibling and swaps it in. Needs the database to itself: refuses while the
/// daemon runs (it holds `<db>.lock`). `--dry-run` instead measures every
/// table and index and estimates the gain - a read of the whole file, slow
/// on a large database but harmless, so it works while the daemon runs.
pub(crate) async fn compact(args: &[String]) -> Result<(), CliError> {
    let Flags {
        dry_run,
        purge_removed,
        keep,
    } = Flags::parse(args)?;
    let (config, store) = open_database().await?;
    let path = config.server.database_path.clone();
    println!("hora compact - {path}");
    if dry_run {
        return report(&store, &path, purge_removed).await;
    }
    // Taken before anything writes, and held to the end.
    let Some(lock) = take_lock(&path)? else {
        return Err(CliError::Failed(
            "An in-memory database has nothing to compact.".to_owned(),
        ));
    };

    let size = store.size().await?;
    println!(
        "  file              {} ({} in free pages)",
        bytes(size.db_bytes),
        bytes(size.free_bytes)
    );
    // The copy holds at most the pages in use.
    let needed = with_margin(size.db_bytes.saturating_sub(size.free_bytes));
    if let Some(free) = print_free_disk(&path, needed)
        && free < needed
    {
        return Err(CliError::Failed(format!(
            "Not enough free disk for the copy: {} free, up to ~{} needed next to the \
             database.",
            bytes(free),
            bytes(needed)
        )));
    }

    println!(
        "Applying the retention{}...",
        if purge_removed {
            " (and deleting the history of monitors removed from the config)"
        } else {
            ""
        }
    );
    let started = std::time::Instant::now();
    db::apply_retention(&store, &config, purge_removed)
        .await
        .context("applying the retention")?;
    println!("  done in {}s", started.elapsed().as_secs());

    println!("Rewriting the file (VACUUM INTO a copy, then swap)...");
    let done = db::vacuum_swap(&lock, store, &path, keep).await?;
    println!(
        "  {} -> {} ({} reclaimed) in {}s",
        bytes(done.bytes_before),
        bytes(done.bytes_after),
        bytes(done.bytes_before.saturating_sub(done.bytes_after)),
        done.elapsed.as_secs()
    );
    if let Some(backup) = done.backup {
        println!(
            "  the original is kept as {} - delete it once the daemon runs fine",
            backup.display()
        );
    }
    Ok(())
}

/// `--dry-run`: what every table and index takes, and what a rewrite would
/// give back. Nothing is written.
async fn report(store: &db::Store, path: &str, purge_removed: bool) -> Result<(), CliError> {
    println!("Measuring every table and index (reads the whole file)...");
    let report = db::size_report(store).await?;
    print_report(&report);
    print_free_disk(path, with_margin(report.estimated_bytes_after));
    println!();
    println!(
        "Dry run: nothing changed. The retention pass of a real run may free more \
         (rows past their window{}).",
        if purge_removed {
            ", and the removed monitors' history"
        } else {
            ""
        }
    );
    Ok(())
}

fn with_margin(bytes: u64) -> u64 {
    bytes.saturating_mul(100 + DISK_MARGIN_PERCENT) / 100
}

/// Print the free disk next to the database against what the copy needs;
/// returns it when the platform can tell.
fn print_free_disk(path: &str, needed: u64) -> Option<u64> {
    let free = db::free_disk_bytes(std::path::Path::new(path));
    match free {
        Some(free) => println!(
            "  free disk         {} (the copy needs ~{})",
            bytes(free),
            bytes(needed)
        ),
        None => println!("  free disk         unknown on this platform"),
    }
    free
}

/// The command-line switches.
struct Flags {
    dry_run: bool,
    purge_removed: bool,
    keep: bool,
}

impl Flags {
    fn parse(args: &[String]) -> Result<Self, CliError> {
        let mut flags = Self {
            dry_run: false,
            purge_removed: false,
            keep: false,
        };
        for arg in args {
            match arg.as_str() {
                "--dry-run" => flags.dry_run = true,
                "--purge-removed" => flags.purge_removed = true,
                "--keep" => flags.keep = true,
                _ => {
                    return Err(usage(
                        "Usage: hora compact [--dry-run] [--purge-removed] [--keep]",
                    ));
                }
            }
        }
        Ok(flags)
    }
}

/// The database to ourselves, or the reason why not.
fn take_lock(path: &str) -> Result<Option<db::DbLock>, CliError> {
    match db::lock_exclusive(path) {
        Ok(lock) => Ok(lock),
        Err(LockError::Busy(lock)) => Err(CliError::Failed(format!(
            "The database is in use: {} is held by the Hora daemon (or another \
             hora compact). Stop the daemon first, then run hora compact again. \
             (hora compact --dry-run works while it runs.)",
            lock.display()
        ))),
        Err(err) => Err(anyhow::Error::new(err).into()),
    }
}

fn print_report(report: &SizeReport) {
    let size = report.size;
    println!("  file              {}", bytes(size.db_bytes));
    println!("  free pages        {}", bytes(size.free_bytes));
    match &report.objects {
        Some(objects) => {
            println!("  tables and indexes:");
            for object in objects.iter().filter(|object| object.bytes >= 1024 * 1024) {
                let what = if object.index {
                    format!("index on {}", object.table)
                } else {
                    "table".to_owned()
                };
                println!(
                    "    {:<34} {:<22} {:>10}  ({} unused)",
                    object.name,
                    what,
                    bytes(object.bytes),
                    bytes(object.unused_bytes)
                );
            }
            let small = objects
                .iter()
                .filter(|object| object.bytes < 1024 * 1024)
                .count();
            if small > 0 {
                println!("    ({small} more under 1 MiB)");
            }
        }
        None => println!("  (this SQLite has no dbstat: free pages only)"),
    }
    let gain = report.estimated_gain();
    println!(
        "  after a rewrite   ~{} (~{} reclaimed, {})",
        bytes(report.estimated_bytes_after),
        bytes(gain),
        hora_core::fmt::pct(
            i64::try_from(gain).unwrap_or(i64::MAX),
            i64::try_from(size.db_bytes.max(1)).unwrap_or(i64::MAX)
        )
    );
}
