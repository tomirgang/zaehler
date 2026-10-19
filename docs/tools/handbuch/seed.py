"""Beispielhaushalt „Musterhaus“ für die Bilder des Benutzerhandbuchs.

Nur gegen eine leere Demo-Instanz mit Entwicklungs-Login ausführen, nie gegen
echte Daten:

    ZAEHLER_AUTH__DEV_LOGIN=true ZAEHLER_SERVER__LISTEN=127.0.0.1:18080 \
      ZAEHLER_SERVER__PUBLIC_URL=http://localhost:18080 \
      ZAEHLER_DATABASE__URL='sqlite:///tmp/zdemo/z.db?mode=rwc' \
      ZAEHLER_STORAGE__PATH=/tmp/zdemo/files target/debug/zaehler serve
    python3 docs/tools/handbuch/seed.py
    (cd tests/browser && node ../../docs/tools/handbuch/shots.mjs)

Umgebung: ZAEHLER_URL (Standard http://localhost:18080), ZAEHLER_DEMO_DB
(SQLite-Datei der Demo; setzt die Quelle der Beispielablesungen auf
„Formular“, damit die Bilder nicht überall „API“ zeigen).
"""
import json, os, re, sqlite3, urllib.request, urllib.parse, http.cookiejar, math
from datetime import date, datetime, timezone, timedelta
BASE = os.environ.get('ZAEHLER_URL', 'http://localhost:18080').rstrip('/')
HERE = os.path.dirname(os.path.abspath(__file__))
jar = http.cookiejar.CookieJar()
op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k): return None
def web(path, form=None):
    data = urllib.parse.urlencode(form).encode() if form else None
    try:
        with op.open(urllib.request.Request(BASE + path, data=data)) as r: return r.read().decode()
    except urllib.error.HTTPError as e: return e.read().decode()
web('/auth/dev-login?name=anna')
html = web('/settings')
csrf = re.search(r'name="_csrf" value="([^"]+)"', html).group(1)
html = web('/settings/tokens', {'_csrf': csrf, 'name': 'Demo', 'role': 'owner', 'expires_on': ''})
TOKEN = re.search(r'value="(zt_[^"]+)"', html).group(1)
def api(method, path, body=None):
    req = urllib.request.Request(BASE + '/api/v1' + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={'Authorization': 'Bearer ' + TOKEN, 'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read(); return json.loads(raw) if raw else None
    except urllib.error.HTTPError as e:
        raise SystemExit(f'{method} {path}: {e.code} {e.read().decode()}\n{json.dumps(body)[:400]}')
def ts(d, h=8): return datetime(d.year, d.month, d.day, h, tzinfo=timezone.utc).isoformat().replace('+00:00', 'Z')
api('PUT', '/household', {'name': 'Musterhaus', 'timezone': 'Europe/Berlin'})
START = date(2025, 1, 1)
def meter(name, kind, regs, **kw):
    body = {'name': name, 'kind': kind, 'decimal_places': kw.pop('decimals', 0),
            'registers': [{'code': c, 'direction': d, 'start_value': str(v)} for c, d, v in regs],
            'start_at': ts(START, 6), **kw}
    return api('POST', '/meters', body)['meter']
house = meter('Hausstrom', 'electricity', [('1.8.0', 'import', 18240), ('2.8.0', 'export', 9120)],
              serial_number='1EMH0012345678', schedule={'kind': 'monthly', 'value': 1}, max_daily_rate='60')
pvm = meter('PV Erzeugung', 'electricity', [('1.8.0', 'import', 4310)], serial_number='SMA-7731', decimals=1)
hp = meter('Wärmepumpe', 'electricity', [('1.8.0', 'import', 6120)], purpose='heat_pump', parent_meter_id=house['id'], serial_number='WP-2291')
wb = meter('Wallbox', 'electricity', [('1.8.0', 'import', 1180)], purpose='wallbox', parent_meter_id=house['id'], decimals=1)
water = meter('Wasser', 'water', [('main', 'import', 412)], serial_number='W-88213', decimals=1, schedule={'kind': 'monthly', 'value': 1})
months = []
d = START
while d <= date(2026, 10, 1):
    months.append(d); d = date(d.year + (d.month == 12), d.month % 12 + 1, 1)
PV_SHARE = [2.5, 4.5, 8, 10.5, 12.5, 13, 13, 11.5, 9, 6.5, 3.5, 2]  # % des Jahres
HEAT = [17, 14, 12, 8, 4, 2, 2, 2, 4, 8, 12, 15]
def cum(start, per_month):
    v, out = start, []
    for i, m in enumerate(months):
        if i: v += per_month(months[i - 1])
        out.append((m, round(v, 1)))
    return out
pv = cum(4310, lambda m: 9600 * PV_SHARE[m.month - 1] / 100 * (1.03 if m.year == 2026 else 1))
imp = cum(18240, lambda m: 4800 * (HEAT[m.month - 1] + 4) / 145)
exp_ = cum(9120, lambda m: 9600 * PV_SHARE[m.month - 1] / 100 * 0.58)
hpv = cum(6120, lambda m: 3400 * HEAT[m.month - 1] / 100)
wbv = cum(1180, lambda m: 1900 / 12 * (0.8 if m.month in (6, 7, 8) else 1.05))
wat = cum(412, lambda m: 110 / 12 * (1.2 if m.month in (6, 7, 8) else 0.95))
def rec(mid, regs, series):
    for i in range(1, len(series[0])):
        at = series[0][i][0]
        api('POST', f'/meters/{mid}/readings', {'read_at': ts(at), 'confirm': True,
            'values': [{'register_id': r, 'value': str(round(s[i][1]))} for r, s in zip(regs, series)]})
rid = lambda m, i=0: m['registers'][i]['id']
rec(house['id'], [rid(house, 0), rid(house, 1)], [imp, exp_])
def rec1(m, series, dec=1):
    for at, v in series[1:]:
        api('POST', f"/meters/{m['id']}/readings", {'read_at': ts(at), 'confirm': True,
            'values': [{'register_id': rid(m), 'value': str(round(v, dec) if dec else round(v))}]})
rec1(pvm, pv); rec1(hp, hpv, 0); rec1(wb, wbv); rec1(water, wat)
# Verträge
def contract(mid, direction, provider, positions, **kw):
    return api('POST', f'/meters/{mid}/contracts', {'direction': direction, 'provider': provider, 'starts_on': '2025-01-01', 'positions': positions, **kw})
pos = lambda name, base, price, interval='year': {'name': name, 'components': [{'valid_from': '2025-01-01', 'base_price': str(base), 'base_interval': interval, 'unit_pricing': {'kind': 'flat', 'price': str(price)}}]}
c1 = contract(house['id'], 'import', 'Stadtwerke Musterstadt', [pos('Strom', 156, '0.3190')], contract_number='SW-4711-22')
api('POST', f"/contracts/{c1['id']}/installments", {'valid_from': '2025-01-01', 'amount': '125', 'every_months': 1})
contract(house['id'], 'export', 'Netzbetreiber Musterland', [pos('Einspeisevergütung', 0, '0.0811')])
c3 = contract(water['id'], 'import', 'Wasserversorgung Musterstadt', [pos('Frischwasser', 6, '2.35', 'month'), pos('Abwasser', 0, '2.80')])
api('POST', f"/contracts/{c3['id']}/installments", {'valid_from': '2025-01-01', 'amount': '45', 'every_months': 1})
# PV-Anlage
api('POST', '/pv-systems', {'name': 'Dach Süd', 'kind': 'surplus', 'kwp': '9.8', 'commissioned_on': '2023-04-01', 'azimuth': 180, 'tilt': 30,
    'inverter': 'SMA Sunny Tripower 10', 'meters': [{'meter_id': house['id'], 'role': 'grid'}, {'meter_id': pvm['id'], 'role': 'generation'}]})
# Fahrzeuge
ev = api('POST', '/vehicles', {'name': 'VW ID.3', 'plate': 'M-ZA 300E', 'model': 'ID.3 Pro', 'drive_type': 'electric',
    'home_charging_meter_id': wb['id'], 'odometer': '18400', 'start_at': ts(START, 6)})
evid = ev.get('meter_id') or ev.get('id') or ev['meter']['id']
car = api('POST', '/vehicles', {'name': 'Skoda Octavia', 'plate': 'M-ZA 1975', 'model': 'Octavia Combi 2.0 TDI', 'drive_type': 'combustion',
    'odometer': '96200', 'start_at': ts(START, 6)})
carid = car.get('meter_id') or car.get('id') or car['meter']['id']
for vid, start, per in ((evid, 18400, 1050), (carid, 96200, 1250)):
    km = start
    for m in months[1:]:
        km += per * (1.3 if m.month == 8 else 1)
        api('POST', f'/meters/{vid}/readings', {'read_at': ts(m), 'confirm': True,
            'values': [{'register_id': api('GET', f'/meters/{vid}')['meter']['registers'][0]['id'], 'value': str(round(km))}]})
api('POST', f'/vehicles/{evid}/profiles', {'valid_from': '2025-01-01', 'energy': 'electric', 'kwh_per_100km': '16.5', 'charging_loss_pct': '10'})
api('PUT', f'/vehicles/{evid}/acquisition', {'purchased_on': '2023-03-15', 'price': '39900', 'mode': 'time', 'first_registration': '2023-03-15', 'residual_value': '16000', 'planned_end': '2031-03-15'})
api('PUT', f'/vehicles/{carid}/acquisition', {'purchased_on': '2019-06-01', 'price': '21500', 'mode': 'mileage', 'first_registration': '2017-04-20', 'odometer_at_purchase': '42000', 'lifetime_km': '300000'})
for at, kwh, eur, prov, kind in (('2025-04-12', 38.2, 21.40, 'Ionity Allersberg', 'dc'), ('2025-08-09', 52.0, 31.20, 'EnBW Rastplatz', 'dc'),
                                 ('2026-02-20', 22.5, 9.90, 'Arbeitgeber', 'ac'), ('2026-07-28', 47.3, 27.40, 'Ionity Kassel', 'dc')):
    api('POST', f'/vehicles/{evid}/charging', {'charged_at': at + 'T15:30:00Z', 'energy': str(kwh), 'amount': str(eur), 'provider': prov, 'kind': kind, 'confirm': True})
def cost(vid, on, cat, amount=None, **kw):
    body = {'booked_on': on, 'category': cat, **kw}
    if amount is not None: body['amount'] = str(amount)
    api('POST', f'/vehicles/{vid}/costs', body)
cost(evid, '2025-01-15', 'insurance', 612.40, recurrence={'every_months': 12, 'until': None}, note='HUK Vollkasko')
cost(evid, '2025-05-20', 'tires', 389.00, note='Sommerreifen')
cost(evid, '2026-03-02', 'inspection', 214.50, note='Inspektion 2 Jahre')
cost(evid, '2025-06-10', 'parking', 14.00)
cost(carid, '2025-01-10', 'insurance', 438.90, recurrence={'every_months': 12, 'until': None})
cost(carid, '2025-06-02', 'tax', 238.00, recurrence={'every_months': 12, 'until': None}, note='Kfz-Steuer')
cost(carid, '2025-09-18', 'workshop', 864.30, note='Zahnriemen')
price = 1.69
for i, m in enumerate(months[1:]):
    on = (m - timedelta(days=9)).isoformat()
    cost(carid, on, 'fuel', quantity=str(round(52 + (i % 4) * 3.5, 2)), unit_price=str(round(price + 0.04 * math.sin(i), 3)), full_tank=True)
api('POST', f"/pv-systems/{api('GET', '/pv-systems')[0]['id']}/costs",
    {'booked_on': '2023-04-01', 'category': 'investment', 'amount': '16400', 'note': 'Anlage inkl. Montage'})
# eine Korrektur für den Änderungsverlauf
r = [x for x in api('GET', f"/meters/{water['id']}/readings?limit=1000")['items'] if x['read_at'].startswith('2026-09-01')][0]
api('PUT', f"/readings/{r['id']}", {'read_at': r['read_at'], 'value': str(round(float(r['value']) - 0.4, 1)),
    'note': 'Ziffer falsch abgelesen', 'confirm': True})
meters = {m['name']: m['id'] for m in api('GET', '/meters')}
ids = {'meters': meters, 'pv': api('GET', '/pv-systems')[0]['id'],
       'contract': next(c['id'] for c in api('GET', '/contracts') if c['provider'].startswith('Stadtwerke'))}
json.dump(ids, open(os.path.join(HERE, 'ids.json'), 'w'), ensure_ascii=False, indent=1)
if db := os.environ.get('ZAEHLER_DEMO_DB'):
    with sqlite3.connect(db) as c:
        c.execute("update readings set source='manual' where source='api'")
print('Beispielhaushalt angelegt')
