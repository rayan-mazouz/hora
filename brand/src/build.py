import sys, os
sys.path.insert(0, os.path.dirname(__file__))
OUT = os.path.join(os.path.dirname(__file__), '..')
import status
pages = {
    'status-ok.html': status.page_ok, 'status-incident.html': status.page_incident,
    'status-maintenance.html': status.page_maint, 'status-local.html': status.page_local, 'status-scale.html': status.page_scale,
}
try:
    import pages2
    pages.update(pages2.PAGES)
except ImportError as e:
    print('pages2 missing', e)
for name, fn in pages.items():
    html = fn()
    with open(os.path.join(OUT, name), 'w') as f:
        f.write(html)
    print(name, len(html.encode()) // 1024, 'KB')
