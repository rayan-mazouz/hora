//! `hora top`: a live terminal dashboard over the JSON API - statuses,
//! uptime, latency percentiles, a sparkline for the selected monitor, and
//! the current trouble, refreshed in place. Self-hosters live in SSH; this
//! is the status page for them. Read-only: it consumes `/api/summary` and
//! `/api/monitors/{id}/latency` exactly like any other API client, local or
//! remote (`--url https://status.example --token ...`).
//!
//! The UI never waits on the network: every request runs as a background
//! task that reports through a channel, so a slow or unreachable server
//! leaves the screen live and `q`/Ctrl-C always answer.

use std::io::IsTerminal as _;
use std::time::Duration;

use anyhow::Context as _;
use crossterm::event::KeyCode;
use futures_util::StreamExt as _;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Sparkline, Table, TableState};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Per-request deadline: a hung server costs one stale round, never the UI.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to back off after the server rate-limited us (HTTP 429).
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(15);

/// What `hora top` reads from `/api/summary` (unknown fields ignored, so the
/// dashboard tolerates both older and newer servers).
#[derive(Deserialize)]
struct Summary {
    title: String,
    /// The worst monitor status (`up`/`degraded`/`down`/`unknown`): drives
    /// the header colour, while `overall_label` is only the wording.
    #[serde(default)]
    overall: String,
    overall_label: String,
    monitors: Vec<Monitor>,
    /// The pinned status-page banners (config + `hora announce`).
    #[serde(default)]
    incidents: Vec<Banner>,
    /// Notification-channel health (authenticated API only; empty/absent on
    /// older or unauthenticated servers).
    #[serde(default)]
    channels: Vec<Channel>,
}

#[derive(Deserialize)]
struct Banner {
    title: String,
    severity: String,
}

#[derive(Deserialize)]
struct Channel {
    name: String,
    #[serde(default)]
    failing: bool,
    #[serde(default)]
    consecutive_failures: u32,
    #[serde(default)]
    failing_for_secs: Option<u64>,
}

#[derive(Deserialize)]
struct Monitor {
    id: String,
    name: String,
    status: String,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default, rename = "uptime_24h_permille")]
    uptime_permille: Option<i64>,
    #[serde(default, rename = "latency_p50_ms")]
    p50: Option<i64>,
    #[serde(default, rename = "latency_p95_ms")]
    p95: Option<i64>,
    #[serde(default, rename = "latency_p99_ms")]
    p99: Option<i64>,
}

#[derive(Deserialize)]
struct Point {
    latency_ms: i64,
}

/// What the input bar is collecting, when open.
enum Input {
    /// `a`: an announcement line - `title :: body`, plus optional
    /// `--severity` / `--until` flags, exactly like the CLI.
    Announce { buffer: String },
    /// `s`: a silence duration for the selected monitor (pre-filled `10m`).
    Silence { monitor_id: String, buffer: String },
}

impl Input {
    fn buffer_mut(&mut self) -> &mut String {
        match self {
            Self::Announce { buffer } | Self::Silence { buffer, .. } => buffer,
        }
    }
}

/// Everything the draw pass needs, updated as fetch results arrive.
struct App {
    url: String,
    summary: Option<Summary>,
    /// 24h latency series of the selected monitor, oldest first.
    spark: Vec<u64>,
    table: TableState,
    /// The last fetch error, shown without wiping the previous data.
    error: Option<String>,
    updated: String,
    /// The input bar, replacing the footer while open.
    input: Option<Input>,
    /// Outcome of the last action: `Ok("pinned ...")` or the reason it failed.
    notice: Option<Result<String, String>>,
    /// Back-off deadline after the server rate-limited us (HTTP 429).
    cooldown_until: Option<tokio::time::Instant>,
}

impl App {
    fn new(url: String) -> Self {
        let mut table = TableState::default();
        table.select(Some(0));
        Self {
            url,
            summary: None,
            spark: Vec::new(),
            table,
            error: None,
            updated: "-".to_owned(),
            input: None,
            notice: None,
            cooldown_until: None,
        }
    }

    fn selected(&self) -> Option<&Monitor> {
        let summary = self.summary.as_ref()?;
        summary.monitors.get(self.table.selected()?)
    }

    fn selected_id(&self) -> Option<String> {
        self.selected().map(|monitor| monitor.id.clone())
    }

    fn select_delta(&mut self, delta: i64) {
        let count = self.summary.as_ref().map_or(0, |s| s.monitors.len());
        if count == 0 {
            return;
        }
        let current = i64::try_from(self.table.selected().unwrap_or(0)).unwrap_or(0);
        let next = (current + delta).rem_euclid(i64::try_from(count).unwrap_or(1));
        self.table.select(Some(usize::try_from(next).unwrap_or(0)));
    }

    /// Whether a 429 back-off is still running (clearing it once expired).
    fn cooling_down(&mut self) -> bool {
        match self.cooldown_until {
            Some(until) if tokio::time::Instant::now() < until => true,
            _ => {
                self.cooldown_until = None;
                false
            }
        }
    }
}

/// Why a request to the API failed - typed, so the loop reacts to the status
/// (a 429 backs off) rather than to the wording of a message.
#[derive(Debug)]
enum FetchError {
    /// The server answered with a non-success status; `detail` is the start
    /// of its body (the API's reason for a refused write).
    Status {
        status: reqwest::StatusCode,
        detail: String,
    },
    /// No answer: connection refused, DNS, timeout.
    Transport(String),
    /// An answer that is not the JSON we expect.
    Shape,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status { status, .. } => write!(f, "server answered HTTP {status}"),
            Self::Transport(reason) => write!(f, "request failed: {reason}"),
            Self::Shape => f.write_str("unexpected response shape"),
        }
    }
}

impl FetchError {
    fn is_rate_limited(&self) -> bool {
        matches!(self, Self::Status { status, .. } if *status == reqwest::StatusCode::TOO_MANY_REQUESTS)
    }

    /// The wording for a refused action: the API's own reason when it gave
    /// one ("server refused (400 Bad Request): invalid duration").
    fn refusal(&self) -> String {
        match self {
            Self::Status { status, detail } => format!("server refused ({status}): {detail}"),
            other => other.to_string(),
        }
    }
}

/// The API endpoint plus credentials; cheap to clone into each fetch task
/// (`reqwest::Client` is reference-counted).
#[derive(Clone)]
struct Api {
    client: reqwest::Client,
    url: String,
    token: Option<String>,
}

impl Api {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let request = self
            .client
            .request(method, format!("{}{path}", self.url.trim_end_matches('/')))
            .timeout(REQUEST_TIMEOUT);
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    /// Send `request`, turning a non-success status into [`FetchError::Status`].
    async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response, FetchError> {
        // The reqwest error embeds the URL (which may carry nothing secret
        // here, but stay consistent with the daemon's logging policy).
        let response = request
            .send()
            .await
            .map_err(|err| FetchError::Transport(err.without_url().to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let detail = response.text().await.unwrap_or_default();
        Err(FetchError::Status {
            status,
            detail: detail.chars().take(120).collect(),
        })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, FetchError> {
        Self::send(self.request(reqwest::Method::GET, path))
            .await?
            .json::<T>()
            .await
            .map_err(|_| FetchError::Shape)
    }
}

/// One authenticated write, built from the keyboard and sent in the
/// background.
#[derive(Debug, PartialEq, Eq)]
struct Action {
    method: reqwest::Method,
    path: &'static str,
    params: Vec<(&'static str, String)>,
    /// The notice shown once the server accepted it.
    done: String,
}

/// A finished background fetch, delivered to the event loop.
enum Fetched {
    Summary(Result<Summary, FetchError>),
    /// The sparkline of `monitor_id` - dropped if the selection moved on.
    Spark {
        monitor_id: String,
        result: Result<Vec<u64>, FetchError>,
    },
    /// An action's outcome, as the notice to show.
    Action(Result<String, String>),
}

/// What the event loop should start after applying a [`Fetched`].
#[derive(Debug, PartialEq, Eq)]
enum FollowUp {
    Nothing,
    /// Fetch this monitor's sparkline.
    Spark(String),
    /// Refetch the summary (an action changed server state).
    Refresh,
}

/// Starts the background fetches, at most one in flight per kind: a slow
/// server never accumulates a queue of summary requests behind it, and a new
/// selection replaces (aborts) the previous sparkline fetch.
struct Fetcher {
    api: Api,
    tx: mpsc::Sender<Fetched>,
    summary: Option<JoinHandle<()>>,
    spark: Option<JoinHandle<()>>,
    action: Option<JoinHandle<()>>,
}

fn in_flight(task: Option<&JoinHandle<()>>) -> bool {
    task.is_some_and(|task| !task.is_finished())
}

impl Fetcher {
    fn new(api: Api, tx: mpsc::Sender<Fetched>) -> Self {
        Self {
            api,
            tx,
            summary: None,
            spark: None,
            action: None,
        }
    }

    /// Fetch `/api/summary` unless a fetch is already running; `false` when
    /// skipped.
    fn summary(&mut self) -> bool {
        if in_flight(self.summary.as_ref()) {
            return false;
        }
        let (api, tx) = (self.api.clone(), self.tx.clone());
        self.summary = Some(tokio::spawn(async move {
            let result = api.get::<Summary>("/api/summary").await;
            let _ = tx.send(Fetched::Summary(result)).await;
        }));
        true
    }

    /// Fetch the 24h latency series of `monitor_id`, abandoning any older
    /// sparkline fetch (its monitor is no longer the selected one).
    fn spark(&mut self, monitor_id: String) {
        if let Some(previous) = self.spark.take() {
            previous.abort();
        }
        let (api, tx) = (self.api.clone(), self.tx.clone());
        self.spark = Some(tokio::spawn(async move {
            let path = format!("/api/monitors/{monitor_id}/latency?hours=24");
            let result = api.get::<Vec<Point>>(&path).await.map(|points| {
                points
                    .iter()
                    .map(|point| u64::try_from(point.latency_ms.max(0)).unwrap_or(0))
                    .collect()
            });
            let _ = tx.send(Fetched::Spark { monitor_id, result }).await;
        }));
    }

    /// Send an action in the background. Refused up front (with the notice
    /// to show) without a token, or while the previous action is in flight.
    fn action(&mut self, action: Action) -> Result<(), String> {
        if self.api.token.is_none() {
            return Err(
                "this action needs a token: run hora top --token ... (or HORA_TOKEN)".to_owned(),
            );
        }
        if in_flight(self.action.as_ref()) {
            return Err("still sending the previous action - try again in a moment".to_owned());
        }
        let (api, tx) = (self.api.clone(), self.tx.clone());
        self.action = Some(tokio::spawn(async move {
            let request = api
                .request(action.method, action.path)
                .query(&action.params);
            let outcome = Api::send(request)
                .await
                .map(|_| action.done)
                .map_err(|err| err.refusal());
            let _ = tx.send(Fetched::Action(outcome)).await;
        }));
        Ok(())
    }
}

impl Drop for Fetcher {
    /// Leaving the dashboard abandons whatever is still in flight.
    fn drop(&mut self) {
        for task in [&self.summary, &self.spark, &self.action]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
    }
}

/// Parse `hora top` arguments and run the dashboard until `q`/`Esc`.
pub async fn run(args: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::io::stdout().is_terminal(),
        "hora top needs an interactive terminal"
    );
    let (url, token, interval) = parse_args(args)?;
    let client = hora_core::http::client(None).context("building HTTP client")?;
    let api = Api { client, url, token };

    // `ratatui::init` enters the alternate screen, enables raw mode, and
    // installs a panic hook that restores the terminal - a crash never
    // leaves the shell in raw mode.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, api, interval).await;
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    api: Api,
    interval: Duration,
) -> anyhow::Result<()> {
    let mut app = App::new(api.url.clone());
    // At most one fetch of each kind is in flight, so a handful of slots is
    // all the channel ever holds.
    let (tx, mut results) = mpsc::channel(8);
    let mut fetcher = Fetcher::new(api, tx);

    let mut events = crossterm::event::EventStream::new();
    // The first tick fires at once: the initial fetch, without delaying the
    // first frame.
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Selection changes only schedule a sparkline fetch: held-down arrow keys
    // auto-repeat tens of times a second, and one request per repeat would
    // burn through the server's per-IP rate limit (the burst is 30 by
    // default). The fetch fires once the selection rests for a moment.
    let mut spark_due: Option<tokio::time::Instant> = None;

    loop {
        terminal.draw(|frame| draw(frame, &mut app))?;
        let spark_timer = async {
            match spark_due {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = ticker.tick() => {
                if !app.cooling_down() {
                    fetcher.summary();
                }
            }
            () = spark_timer => {
                spark_due = None;
                if let Some(id) = app.selected_id() {
                    fetcher.spark(id);
                }
            }
            // The fetcher holds a sender, so the channel never closes.
            Some(done) = results.recv() => match apply(&mut app, done) {
                FollowUp::Nothing => {}
                FollowUp::Spark(id) => fetcher.spark(id),
                FollowUp::Refresh => {
                    fetcher.summary();
                }
            },
            event = events.next() => {
                let Some(Ok(crossterm::event::Event::Key(key))) = event else { continue };
                match on_key(&mut app, &mut fetcher, key) {
                    KeyOutcome::Quit => return Ok(()),
                    KeyOutcome::Moved => {
                        app.spark.clear();
                        spark_due =
                            Some(tokio::time::Instant::now() + Duration::from_millis(300));
                    }
                    KeyOutcome::Handled => {}
                }
            }
        }
    }
}

/// What a key press asks of the event loop.
enum KeyOutcome {
    Quit,
    /// The selection moved: schedule the new monitor's sparkline.
    Moved,
    Handled,
}

/// Handle one key event. Never awaits: anything that talks to the server is
/// handed to the fetcher.
fn on_key(app: &mut App, fetcher: &mut Fetcher, key: crossterm::event::KeyEvent) -> KeyOutcome {
    if key.kind != crossterm::event::KeyEventKind::Press {
        return KeyOutcome::Handled;
    }
    if key
        .modifiers
        .contains(crossterm::event::KeyModifiers::CONTROL)
        && key.code == KeyCode::Char('c')
    {
        return KeyOutcome::Quit;
    }
    // While the input bar is open it owns the keyboard.
    if app.input.is_some() {
        if let Some(action) = input_key(app, key.code) {
            start_action(app, fetcher, action);
        }
        return KeyOutcome::Handled;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return KeyOutcome::Quit,
        KeyCode::Down | KeyCode::Char('j') => {
            app.select_delta(1);
            return KeyOutcome::Moved;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.select_delta(-1);
            return KeyOutcome::Moved;
        }
        KeyCode::Char('r') => {
            if !app.cooling_down() {
                fetcher.summary();
            }
        }
        KeyCode::Char('a') => {
            app.notice = None;
            app.input = Some(Input::Announce {
                buffer: String::new(),
            });
        }
        KeyCode::Char('s') => {
            if let Some(id) = app.selected_id() {
                app.notice = None;
                app.input = Some(Input::Silence {
                    monitor_id: id,
                    buffer: "10m".to_owned(),
                });
            }
        }
        KeyCode::Char('C') => start_action(
            app,
            fetcher,
            Ok(Action {
                method: reqwest::Method::DELETE,
                path: "/api/announce",
                params: Vec::new(),
                done: "announcements cleared".to_owned(),
            }),
        ),
        _ => {}
    }
    KeyOutcome::Handled
}

/// Fold a finished fetch into the app. A failure keeps the previous data on
/// screen and surfaces the reason in the header; a 429 backs off instead of
/// hammering the server's rate limit further.
fn apply(app: &mut App, fetched: Fetched) -> FollowUp {
    match fetched {
        Fetched::Summary(Ok(summary)) => {
            // Keep the selection on the same monitor across refreshes.
            if let Some(id) = app.selected_id() {
                let index = summary.monitors.iter().position(|m| m.id == id);
                app.table.select(index.or(Some(0)));
            }
            app.summary = Some(summary);
            app.error = None;
            app.updated = chrono::Utc::now().format("%H:%M:%S UTC").to_string();
            app.selected_id().map_or(FollowUp::Nothing, FollowUp::Spark)
        }
        Fetched::Summary(Err(err)) => {
            if err.is_rate_limited() {
                app.cooldown_until = Some(tokio::time::Instant::now() + RATE_LIMIT_COOLDOWN);
                app.error =
                    Some("rate limited by the server (HTTP 429) - backing off 15s".to_owned());
            } else {
                app.error = Some(err.to_string());
            }
            FollowUp::Nothing
        }
        Fetched::Spark { monitor_id, result } => {
            // A late answer for a monitor no longer selected is stale.
            if app.selected_id().as_deref() == Some(monitor_id.as_str()) {
                app.spark = result.unwrap_or_default();
            }
            FollowUp::Nothing
        }
        Fetched::Action(outcome) => {
            app.notice = Some(outcome);
            FollowUp::Refresh
        }
    }
}

/// Start `action` (or show why it could not be built or sent).
fn start_action(app: &mut App, fetcher: &mut Fetcher, action: Result<Action, String>) {
    app.notice = match action.and_then(|action| fetcher.action(action)) {
        Ok(()) => None,
        Err(reason) => Some(Err(reason)),
    };
}

/// Keys while the input bar is open: type, Backspace, Enter submits (closing
/// the bar and yielding the action, or why it is invalid), Esc cancels.
fn input_key(app: &mut App, code: KeyCode) -> Option<Result<Action, String>> {
    match code {
        KeyCode::Enter => match app.input.take()? {
            Input::Announce { buffer } => Some(announce_action(&buffer)),
            Input::Silence { monitor_id, buffer } => Some(silence_action(&monitor_id, &buffer)),
        },
        KeyCode::Esc => {
            app.input = None;
            None
        }
        KeyCode::Backspace => {
            app.input.as_mut()?.buffer_mut().pop();
            None
        }
        KeyCode::Char(c) => {
            app.input.as_mut()?.buffer_mut().push(c);
            None
        }
        _ => None,
    }
}

/// Pin an announcement from the input line: `title :: body`, with optional
/// `--severity <s>` and `--until <4h|18:00-style duration>` flags anywhere -
/// the same grammar as `hora announce`.
fn announce_action(line: &str) -> Result<Action, String> {
    let mut severity = "warning"; // announcing from `top` usually means trouble
    let mut until = None;
    let mut words: Vec<&str> = Vec::new();
    let mut iter = line.split_whitespace();
    while let Some(word) = iter.next() {
        match word {
            "--severity" => {
                let value = iter.next().unwrap_or("warning");
                if !matches!(value, "info" | "warning" | "critical" | "resolved") {
                    return Err("severity must be info, warning, critical or resolved".to_owned());
                }
                severity = value;
            }
            "--until" => until = iter.next(),
            other => words.push(other),
        }
    }
    let text = words.join(" ");
    let (title, body) = text
        .split_once("::")
        .map_or((text.as_str(), ""), |(t, b)| (t, b));
    let (title, body) = (title.trim().to_owned(), body.trim().to_owned());
    if title.is_empty() {
        return Err("announce needs a title".to_owned());
    }
    let done = format!("announced [{severity}] {title:?}");
    let mut params = vec![("title", title), ("severity", severity.to_owned())];
    if !body.is_empty() {
        params.push(("body", body));
    }
    if let Some(until) = until {
        params.push(("until", until.to_owned()));
    }
    Ok(Action {
        method: reqwest::Method::POST,
        path: "/api/announce",
        params,
        done,
    })
}

/// Silence the selected monitor for the typed duration.
fn silence_action(monitor_id: &str, duration: &str) -> Result<Action, String> {
    let duration = duration.trim();
    if duration.is_empty() {
        return Err("silence needs a duration (e.g. 10m)".to_owned());
    }
    Ok(Action {
        method: reqwest::Method::POST,
        path: "/api/silence",
        params: vec![
            ("monitors", monitor_id.to_owned()),
            ("duration", duration.to_owned()),
            ("reason", "silenced from hora top".to_owned()),
        ],
        done: format!("silenced {monitor_id} for {duration}"),
    })
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let [header, table_area, spark_area, trouble, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(5),
        Constraint::Length(4),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // Header: title - overall - updated - source.
    let (title, overall, label) = app.summary.as_ref().map_or_else(
        || ("hora".to_owned(), "", "connecting...".to_owned()),
        |summary| {
            (
                summary.title.clone(),
                summary.overall.as_str(),
                summary.overall_label.clone(),
            )
        },
    );
    let mut spans = vec![
        Span::styled(format!(" {title} "), Style::new().bold()),
        Span::styled(format!(" {label} "), overall_style(overall)),
        Span::raw(format!("  updated {}  {}", app.updated, app.url)),
    ];
    if let Some(error) = &app.error {
        spans.push(Span::styled(
            format!("  {error}"),
            Style::new().fg(Color::Red).bold(),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), header);

    frame.render_stateful_widget(monitor_table(app), table_area, &mut app.table);

    // Sparkline of the selected monitor's last 24h.
    let spark_title = app.selected().map_or_else(
        || "latency".to_owned(),
        |monitor| {
            format!(
                "latency 24h - {} (p95 {})",
                monitor.name,
                ms(monitor.p95).trim().to_owned()
            )
        },
    );
    let spark = Sparkline::default()
        .block(
            Block::new()
                .borders(Borders::TOP)
                .title(spark_title)
                .border_style(Style::new().fg(Color::DarkGray)),
        )
        .data(&app.spark)
        .style(Style::new().fg(Color::Cyan));
    frame.render_widget(spark, spark_area);

    frame.render_widget(
        Paragraph::new(trouble_lines(app)).block(
            Block::new()
                .borders(Borders::TOP)
                .title("trouble")
                .border_style(Style::new().fg(Color::DarkGray)),
        ),
        trouble,
    );

    frame.render_widget(Paragraph::new(footer_line(app)), footer);
}

/// The footer: the input bar while it is open, otherwise the key hints plus
/// the outcome of the last action.
fn footer_line(app: &App) -> Line<'static> {
    if let Some(input) = &app.input {
        let (label, buffer) = match input {
            Input::Announce { buffer } => (
                " announce: title :: body [--severity info|warning|critical|resolved] [--until 4h|18:00] "
                    .to_owned(),
                buffer,
            ),
            Input::Silence { monitor_id, buffer } => {
                (format!(" silence {monitor_id} for: "), buffer)
            }
        };
        return Line::from(vec![
            Span::styled(label, Style::new().fg(Color::Black).bg(Color::Cyan)),
            Span::raw(format!(" {buffer}")),
            Span::styled("█", Style::new().fg(Color::Cyan)),
            Span::styled(
                "  (Enter sends · Esc cancels)",
                Style::new().fg(Color::DarkGray),
            ),
        ]);
    }
    let mut spans = vec![Span::styled(
        " q quit · ↑/↓ select · r refresh · a announce · s silence · C clear banners",
        Style::new().fg(Color::DarkGray),
    )];
    if let Some(notice) = &app.notice {
        let (text, style) = match notice {
            Ok(done) => (done, Style::new().fg(Color::Green)),
            Err(reason) => (reason, Style::new().fg(Color::Red).bold()),
        };
        spans.push(Span::styled(format!("   {text}"), style));
    }
    Line::from(spans)
}

/// The monitor table: one row per monitor, coloured by status.
fn monitor_table(app: &App) -> Table<'static> {
    let rows: Vec<Row> = app.summary.as_ref().map_or_else(Vec::new, |summary| {
        summary
            .monitors
            .iter()
            .map(|monitor| {
                Row::new(vec![
                    format!(" {}", status_dot(&monitor.status)),
                    monitor.name.clone(),
                    monitor.group.clone().unwrap_or_default(),
                    monitor
                        .uptime_permille
                        .map_or_else(|| "-".to_owned(), hora_core::fmt::permille),
                    ms(monitor.p50),
                    ms(monitor.p95),
                    ms(monitor.p99),
                    monitor.last_error.clone().unwrap_or_default(),
                ])
                .style(status_style(&monitor.status))
            })
            .collect()
    });
    Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(16),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec![
            "",
            "MONITOR",
            "GROUP",
            "UP 24H",
            "P50",
            "P95",
            "P99",
            "LAST ERROR",
        ])
        .style(Style::new().fg(Color::DarkGray)),
    )
    .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
    .block(
        Block::new()
            .borders(Borders::TOP)
            .border_style(Style::new().fg(Color::DarkGray)),
    )
}

/// Pinned banners first, then every monitor that is not up with its reason -
/// or one green line. Failing notification channels are shown here too: a
/// broken Telegram bot discovered at incident time is the failure this dashboard
/// exists to prevent.
fn trouble_lines(app: &App) -> Vec<Line<'static>> {
    let mut banners: Vec<Line> = app.summary.as_ref().map_or_else(Vec::new, |summary| {
        summary
            .incidents
            .iter()
            .take(2)
            .map(|banner| {
                Line::from(Span::styled(
                    format!(" 📌 [{}] {}", banner.severity, banner.title),
                    severity_style(&banner.severity),
                ))
            })
            .collect()
    });
    let troubled: Vec<Line> = app.summary.as_ref().map_or_else(Vec::new, |summary| {
        summary
            .monitors
            .iter()
            .filter(|monitor| monitor.status != "up")
            .take(3)
            .map(|monitor| {
                Line::from(vec![
                    Span::styled(
                        format!(" {} {} ", status_dot(&monitor.status), monitor.name),
                        status_style(&monitor.status).bold(),
                    ),
                    Span::raw(monitor.last_error.clone().unwrap_or_default()),
                ])
            })
            .collect()
    });
    let broken_channels: Vec<Line> = app.summary.as_ref().map_or_else(Vec::new, |summary| {
        summary
            .channels
            .iter()
            .filter(|ch| ch.failing)
            .map(|ch| {
                Line::from(Span::styled(
                    format!(
                        " ⚠ channel '{}' failing ({} failures, {})",
                        ch.name,
                        ch.consecutive_failures,
                        ch.failing_for_secs
                            .map_or_else(|| "?".to_owned(), hora_core::fmt::elapsed),
                    ),
                    Style::new().fg(Color::Yellow),
                ))
            })
            .collect()
    });
    if troubled.is_empty() && banners.is_empty() && broken_channels.is_empty() {
        return vec![Line::from(Span::styled(
            " all monitors up",
            Style::new().fg(Color::Green),
        ))];
    }
    banners.extend(troubled);
    banners.extend(broken_channels);
    banners
}

fn severity_style(severity: &str) -> Style {
    match severity {
        "critical" => Style::new().fg(Color::Red),
        "warning" => Style::new().fg(Color::Yellow),
        "resolved" => Style::new().fg(Color::Green),
        _ => Style::new().fg(Color::Cyan),
    }
}

fn ms(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_owned(), |ms| format!("{ms}ms"))
}

fn status_dot(status: &str) -> &'static str {
    match status {
        "up" => "●",
        "degraded" => "◐",
        "down" => "○",
        _ => "·",
    }
}

fn status_style(status: &str) -> Style {
    match status {
        "up" => Style::new().fg(Color::Green),
        "degraded" => Style::new().fg(Color::Yellow),
        "down" => Style::new().fg(Color::Red),
        _ => Style::new().fg(Color::DarkGray),
    }
}

/// The header badge colour, from the summary's machine-readable `overall`.
fn overall_style(overall: &str) -> Style {
    match overall {
        "up" => Style::new().fg(Color::Black).bg(Color::Green),
        "degraded" => Style::new().fg(Color::Black).bg(Color::Yellow),
        "down" => Style::new().fg(Color::White).bg(Color::Red),
        _ => Style::new().fg(Color::Black).bg(Color::DarkGray),
    }
}

/// `hora top [--url URL] [--token TOKEN] [--interval SECS]`. Without `--url`
/// the local config's `server.bind` is used; the token also falls back to
/// the `HORA_TOKEN` environment variable (kept out of `ps` output).
fn parse_args(args: &[String]) -> anyhow::Result<(String, Option<String>, Duration)> {
    let mut url = None;
    let mut token = std::env::var("HORA_TOKEN").ok();
    let mut interval = 5_u64;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--url" => url = Some(required(&mut iter, "--url")?),
            "--token" => token = Some(required(&mut iter, "--token")?),
            "--interval" => {
                interval = required(&mut iter, "--interval")?
                    .parse()
                    .context("--interval must be seconds")?;
            }
            other => anyhow::bail!("unknown option {other:?} (try --url, --token, --interval)"),
        }
    }
    let url = if let Some(url) = url {
        url
    } else {
        let config = hora_core::config::load_from(&hora_core::config::path())
            .context("no --url given and the local config did not load")?;
        // A wildcard bind (the Docker default, HORA_BIND=0.0.0.0:8787) is a
        // listen address, not a place to connect to: talk to loopback.
        let bind = config
            .server
            .bind
            .replacen("0.0.0.0", "127.0.0.1", 1)
            .replacen("[::]", "[::1]", 1);
        format!("http://{bind}")
    };
    Ok((url, token, Duration::from_secs(interval.max(1))))
}

fn required(iter: &mut std::slice::Iter<'_, String>, flag: &str) -> anyhow::Result<String> {
    iter.next()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_parse_flags_and_fall_back() {
        let (url, token, interval) = parse_args(&[
            "--url".to_owned(),
            "https://s.example".to_owned(),
            "--token".to_owned(),
            "tok".to_owned(),
            "--interval".to_owned(),
            "2".to_owned(),
        ])
        .expect("parse");
        assert_eq!(url, "https://s.example");
        assert_eq!(token.as_deref(), Some("tok"));
        assert_eq!(interval, Duration::from_secs(2));

        // Unknown flags fail loudly; a zero interval is clamped.
        assert!(parse_args(&["--nope".to_owned()]).is_err());
        let (_, _, interval) = parse_args(&[
            "--url".to_owned(),
            "https://s".to_owned(),
            "--interval".to_owned(),
            "0".to_owned(),
        ])
        .expect("parse");
        assert_eq!(interval, Duration::from_secs(1));
    }

    #[test]
    fn summary_json_from_the_real_api_shape_deserializes() {
        let summary: Summary = serde_json::from_str(
            r#"{
                "title": "uplg.status",
                "overall": "degraded",
                "overall_label": "Degraded performance",
                "incidents": [],
                "maintenances": [],
                "groups": [{"name": "Frank", "ids": ["web"]}],
                "peers": [],
                "monitors": [{
                    "id": "web",
                    "name": "Web",
                    "status": "degraded",
                    "last_latency_ms": 812,
                    "last_error": "slow",
                    "last_checked": "12s ago",
                    "uptime_24h_permille": 998,
                    "latency_p50_ms": 120,
                    "latency_p95_ms": 800,
                    "latency_p99_ms": 950,
                    "history": [],
                    "group": "Frank",
                    "maintenance": null
                }]
            }"#,
        )
        .expect("summary shape");
        assert_eq!(summary.monitors[0].uptime_permille, Some(998));
        assert_eq!(summary.monitors[0].p95, Some(800));
        assert_eq!(summary.overall_label, "Degraded performance");
        // The header colour comes from the status, not from the wording.
        assert_eq!(summary.overall, "degraded");
        assert_eq!(overall_style(&summary.overall).bg, Some(Color::Yellow));
        assert_eq!(overall_style("down").bg, Some(Color::Red));
        assert_eq!(overall_style("").bg, Some(Color::DarkGray));
    }

    fn summary_of(ids: &[&str]) -> Summary {
        let monitors: Vec<String> = ids
            .iter()
            .map(|id| format!(r#"{{"id": "{id}", "name": "{id}", "status": "up"}}"#))
            .collect();
        serde_json::from_str(&format!(
            r#"{{"title": "t", "overall": "up", "overall_label": "All systems operational",
                "monitors": [{}]}}"#,
            monitors.join(",")
        ))
        .expect("summary")
    }

    fn api_at(url: String) -> Api {
        Api {
            client: hora_core::http::client(None).expect("client"),
            url,
            token: Some("tok".to_owned()),
        }
    }

    #[tokio::test]
    async fn summary_keeps_the_selection_and_asks_for_its_sparkline() {
        let mut app = App::new("http://x".to_owned());
        assert_eq!(
            apply(&mut app, Fetched::Summary(Ok(summary_of(&["a", "b"])))),
            FollowUp::Spark("a".to_owned())
        );
        app.select_delta(1);
        // "b" moved to the front: the selection follows the monitor.
        assert_eq!(
            apply(&mut app, Fetched::Summary(Ok(summary_of(&["b", "a"])))),
            FollowUp::Spark("b".to_owned())
        );
        assert_eq!(app.table.selected(), Some(0));
    }

    #[tokio::test]
    async fn a_late_sparkline_for_another_monitor_is_dropped() {
        let mut app = App::new("http://x".to_owned());
        apply(&mut app, Fetched::Summary(Ok(summary_of(&["a", "b"]))));
        apply(
            &mut app,
            Fetched::Spark {
                monitor_id: "b".to_owned(),
                result: Ok(vec![1, 2, 3]),
            },
        );
        assert_eq!(app.spark, Vec::<u64>::new());
        apply(
            &mut app,
            Fetched::Spark {
                monitor_id: "a".to_owned(),
                result: Ok(vec![4, 5]),
            },
        );
        assert_eq!(app.spark, vec![4, 5]);
    }

    #[tokio::test]
    async fn rate_limiting_is_detected_from_the_status_not_the_text() {
        let mut app = App::new("http://x".to_owned());
        let refused = FetchError::Status {
            status: reqwest::StatusCode::TOO_MANY_REQUESTS,
            detail: String::new(),
        };
        assert_eq!(
            apply(&mut app, Fetched::Summary(Err(refused))),
            FollowUp::Nothing
        );
        assert!(app.cooling_down());
        assert!(app.error.as_deref().is_some_and(|e| e.contains("429")));

        // A body that merely mentions 429 is an ordinary error.
        let mut app = App::new("http://x".to_owned());
        let other = FetchError::Status {
            status: reqwest::StatusCode::BAD_GATEWAY,
            detail: "upstream 429".to_owned(),
        };
        apply(&mut app, Fetched::Summary(Err(other)));
        assert!(!app.cooling_down());
        assert_eq!(
            app.error.as_deref(),
            Some("server answered HTTP 502 Bad Gateway")
        );
    }

    #[tokio::test]
    async fn action_outcomes_become_typed_notices_and_refresh() {
        let mut app = App::new("http://x".to_owned());
        assert_eq!(
            apply(&mut app, Fetched::Action(Err("failed somehow".to_owned()))),
            FollowUp::Refresh
        );
        assert_eq!(app.notice, Some(Err("failed somehow".to_owned())));
        // A success whose wording contains "failed" still reads as success.
        apply(
            &mut app,
            Fetched::Action(Ok("announced [info] \"failed disk swapped\"".to_owned())),
        );
        assert!(matches!(app.notice, Some(Ok(_))));
    }

    #[test]
    fn input_bar_builds_actions_without_touching_the_network() {
        let mut app = App::new("http://x".to_owned());
        app.input = Some(Input::Silence {
            monitor_id: "web".to_owned(),
            buffer: "10m".to_owned(),
        });
        assert!(input_key(&mut app, KeyCode::Backspace).is_none());
        assert!(input_key(&mut app, KeyCode::Char('h')).is_none());
        let action = input_key(&mut app, KeyCode::Enter)
            .expect("submitted")
            .expect("valid");
        assert!(app.input.is_none(), "Enter closes the bar");
        assert_eq!(action.path, "/api/silence");
        assert!(action.params.contains(&("duration", "10h".to_owned())));
        // Enter with the bar already closed is a no-op.
        assert!(input_key(&mut app, KeyCode::Enter).is_none());

        let action = announce_action("Fiber cut :: ETA 6pm --severity critical --until 4h")
            .expect("announce");
        assert_eq!(action.done, "announced [critical] \"Fiber cut\"");
        assert!(action.params.contains(&("body", "ETA 6pm".to_owned())));
        assert!(action.params.contains(&("until", "4h".to_owned())));
        assert!(announce_action(":: body only").is_err());
        assert!(announce_action("t --severity panic").is_err());
        assert!(silence_action("web", "  ").is_err());
    }

    /// The UI must never wait on the network: against a server that accepts
    /// connections but never answers, starting a fetch returns at once, and a
    /// second summary fetch is skipped while the first is in flight.
    #[tokio::test]
    async fn fetches_run_in_the_background_one_per_kind() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let (tx, mut results) = mpsc::channel(8);
        let mut fetcher = Fetcher::new(api_at(url), tx);

        assert!(fetcher.summary(), "first fetch starts");
        assert!(!fetcher.summary(), "second is skipped while in flight");
        fetcher
            .action(silence_action("web", "10m").expect("action"))
            .expect("action starts");
        assert!(
            fetcher
                .action(silence_action("web", "10m").expect("action"))
                .is_err(),
            "one action at a time"
        );
        // Nothing has answered, and nothing blocked getting here.
        assert!(
            tokio::time::timeout(Duration::from_millis(100), results.recv())
                .await
                .is_err()
        );
        drop(listener);
    }

    #[tokio::test]
    async fn fetch_errors_carry_the_http_status() {
        let app = axum::Router::new()
            .route(
                "/api/summary",
                axum::routing::get(|| async {
                    (axum::http::StatusCode::TOO_MANY_REQUESTS, "slow down")
                }),
            )
            .route(
                "/api/silence",
                axum::routing::post(|| async {
                    (axum::http::StatusCode::BAD_REQUEST, "invalid duration")
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move { axum::serve(listener, app).await });

        let (tx, mut results) = mpsc::channel(8);
        let mut fetcher = Fetcher::new(api_at(url.clone()), tx);
        fetcher.summary();
        match results.recv().await {
            Some(Fetched::Summary(Err(err))) => assert!(err.is_rate_limited(), "{err}"),
            _ => panic!("expected a failed summary"),
        }
        fetcher
            .action(silence_action("web", "nope").expect("action"))
            .expect("starts");
        match results.recv().await {
            Some(Fetched::Action(Err(reason))) => {
                assert_eq!(reason, "server refused (400 Bad Request): invalid duration");
            }
            _ => panic!("expected a refused action"),
        }

        // Without a token, an action is refused before any request.
        let (tx, _results) = mpsc::channel(8);
        let mut anonymous = Fetcher::new(
            Api {
                token: None,
                ..api_at(url)
            },
            tx,
        );
        let refused = anonymous
            .action(silence_action("web", "10m").expect("action"))
            .expect_err("needs a token");
        assert!(refused.contains("needs a token"));
    }
}
