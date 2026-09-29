#!/usr/bin/env python3
"""Small, dependency-free, read-only SQLite table viewer."""

from __future__ import annotations

import argparse
import json
import sqlite3
import threading
import urllib.parse
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


HTML = r'''<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Vicki · Database viewer</title>
  <style>
    :root{color-scheme:dark;--bg:#0a0d12;--panel:#11151c;--panel2:#171c24;--line:#272e39;--muted:#8e98a7;--text:#edf1f7;--accent:#f4b860;--accent2:#ffcf85;--danger:#ff7d7d;--sans:Inter,ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;--mono:"Cascadia Code",Consolas,monospace}
    *{box-sizing:border-box} body{margin:0;background:var(--bg);color:var(--text);font-family:var(--sans);font-size:15px;min-height:100vh}.shell{display:grid;grid-template-columns:248px minmax(0,1fr);min-height:100vh}.side{border-right:1px solid var(--line);background:#0d1016;padding:24px 16px;position:sticky;top:0;height:100vh}.brand{display:flex;align-items:center;gap:12px;padding:0 8px 26px}.mark{width:34px;height:34px;border:1px solid #57462e;background:linear-gradient(145deg,#2b241b,#161a21);display:grid;place-items:center;border-radius:9px;color:var(--accent);font-family:var(--mono);font-weight:700}.brand strong{display:block;font-size:15px}.brand small{display:block;color:var(--muted);margin-top:2px}.label{text-transform:uppercase;letter-spacing:.12em;color:#697383;font-size:11px;font-weight:700;padding:0 10px 8px}.tables{display:grid;gap:5px}.table-btn{appearance:none;border:0;color:#aeb6c2;background:transparent;text-align:left;padding:11px 12px;border-radius:8px;font:inherit;cursor:pointer;display:flex;align-items:center;justify-content:space-between}.table-btn:hover{background:#151a22;color:var(--text)}.table-btn.active{background:#202630;color:white;box-shadow:inset 3px 0 var(--accent)}.table-btn span:last-child{font:12px var(--mono);color:#727d8d}.db-note{position:absolute;bottom:22px;left:24px;right:20px;color:#687282;font-size:12px;line-height:1.5;overflow-wrap:anywhere}.main{min-width:0;padding:30px 34px 40px}.top{display:flex;justify-content:space-between;gap:24px;align-items:flex-end;margin-bottom:24px}.eyebrow{font:12px var(--mono);color:var(--accent);text-transform:uppercase;letter-spacing:.12em;margin-bottom:7px}h1{font-size:28px;letter-spacing:-.035em;margin:0}.meta{color:var(--muted);margin-top:7px}.readonly{border:1px solid #3a4657;color:#aab4c2;border-radius:999px;padding:7px 11px;font-size:12px;white-space:nowrap}.toolbar{display:flex;gap:10px;margin-bottom:14px}.search{height:42px;min-width:260px;max-width:520px;flex:1;border:1px solid var(--line);background:var(--panel);color:var(--text);border-radius:9px;padding:0 14px;font:inherit;outline:none}.search:focus{border-color:#6d5737;box-shadow:0 0 0 3px #f4b86012}.search::placeholder{color:#6e7785}.select{height:42px;border:1px solid var(--line);background:var(--panel);color:var(--text);border-radius:9px;padding:0 34px 0 12px;font:inherit}.frame{border:1px solid var(--line);background:var(--panel);border-radius:12px;overflow:hidden;box-shadow:0 18px 60px #00000026}.scroll{overflow:auto;max-height:calc(100vh - 245px)}table{border-collapse:separate;border-spacing:0;width:100%;min-width:850px}th{position:sticky;top:0;z-index:2;background:#181d25;color:#99a4b3;text-transform:uppercase;letter-spacing:.07em;font-size:11px;font-weight:700;text-align:left;border-bottom:1px solid var(--line);padding:12px 14px;cursor:pointer;white-space:nowrap}th:hover{color:var(--accent2)}td{padding:11px 14px;border-bottom:1px solid #202630;color:#d8dde5;white-space:nowrap;max-width:390px;overflow:hidden;text-overflow:ellipsis}tbody tr:hover td{background:#181d25}tbody tr:last-child td{border-bottom:0}.num{font-family:var(--mono);font-size:13px;text-align:right}.hash{font-family:var(--mono);font-size:12px;color:#8996a7}.path{font-family:var(--mono);font-size:12.5px}.empty{padding:70px 20px;text-align:center;color:var(--muted)}.foot{height:54px;border-top:1px solid var(--line);display:flex;align-items:center;justify-content:space-between;padding:0 14px;color:var(--muted);font-size:13px}.pages{display:flex;align-items:center;gap:8px}.pages button{height:32px;border:1px solid var(--line);background:var(--panel2);color:var(--text);border-radius:7px;padding:0 12px;cursor:pointer}.pages button:disabled{opacity:.35;cursor:default}.error{border:1px solid #632f35;background:#28171a;color:#ffb5bd;padding:14px;border-radius:9px}.loading td{color:#727d8d;text-align:center;padding:60px}.sort{color:var(--accent);margin-left:5px}
    @media(max-width:760px){.shell{display:block}.side{position:static;height:auto;border:0;border-bottom:1px solid var(--line);padding:14px}.brand{padding:0 0 14px}.tables{display:flex;overflow:auto}.table-btn{min-width:max-content}.label,.db-note{display:none}.main{padding:22px 14px}.top{align-items:flex-start}h1{font-size:24px}.toolbar{flex-wrap:wrap}.search{min-width:100%;}.scroll{max-height:60vh}.readonly{display:none}.foot{height:auto;padding:12px;gap:10px;flex-wrap:wrap}}
  </style>
</head>
<body><div class="shell">
  <aside class="side"><div class="brand"><div class="mark">V</div><div><strong>Vicki</strong><small>Database viewer</small></div></div><div class="label">Tables</div><nav class="tables" id="tables"></nav><div class="db-note" id="dbpath"></div></aside>
  <main class="main"><header class="top"><div><div class="eyebrow">SQLite / master.db</div><h1 id="title">Loading…</h1><div class="meta" id="meta"></div></div><div class="readonly">● Read-only connection</div></header>
    <div class="toolbar"><input class="search" id="search" type="search" placeholder="Search this table…" autocomplete="off"><select class="select" id="pagesize" aria-label="Rows per page"><option>25</option><option selected>50</option><option>100</option><option>250</option></select></div>
    <div id="message"></div><section class="frame"><div class="scroll"><table><thead id="head"></thead><tbody id="body"><tr class="loading"><td>Reading database…</td></tr></tbody></table></div><footer class="foot"><span id="range"></span><div class="pages"><button id="prev">← Previous</button><span id="page"></span><button id="next">Next →</button></div></footer></section>
  </main></div>
<script>
const S={info:null,table:null,page:1,size:50,q:'',sort:null,dir:'desc'}; const $=id=>document.getElementById(id);
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const nice=n=>String(n).replaceAll('_',' ').replace(/\b\w/g,c=>c.toUpperCase());
const bytes=n=>{if(n==null)return '—';let i=0,v=Number(n),u=['B','KB','MB','GB','TB'];while(v>=1024&&i<u.length-1){v/=1024;i++}return `${v.toLocaleString(undefined,{maximumFractionDigits:i?1:0})} ${u[i]}`};
function value(col,v){if(v==null)return '<span style="color:#66717f">NULL</span>';if(col.endsWith('_unix'))return new Date(Number(v)*1000).toLocaleString();if(col.includes('bytes')||col==='file_size')return bytes(v);if(col.includes('blake3'))return `<span class="hash" title="${esc(v)}">${esc(String(v).slice(0,12))}…</span>`;if(col.includes('path')||col==='source_folder')return `<span class="path" title="${esc(v)}">${esc(v)}</span>`;if(col==='include_hidden')return v?'Yes':'No';return esc(v)}
function cellClass(col,type){return /INT|REAL|NUM|DEC|FLOAT|DOUBLE/.test(type)?'num':(col.includes('path')||col==='source_folder'?'path':'')}
async function init(){try{S.info=await (await fetch('/api/info')).json();$('dbpath').textContent=S.info.path;$('tables').innerHTML=S.info.tables.map((t,i)=>`<button class="table-btn" data-table="${esc(t.name)}"><span>${esc(t.name)}</span><span>${t.rows.toLocaleString()}</span></button>`).join('');$('tables').onclick=e=>{let b=e.target.closest('button');if(b)selectTable(b.dataset.table)};selectTable(S.info.tables[0]?.name)}catch(e){fail(e)}}
function selectTable(name){S.table=name;S.page=1;S.sort=null;S.dir='desc';$('search').value='';S.q='';document.querySelectorAll('.table-btn').forEach(b=>b.classList.toggle('active',b.dataset.table===name));load()}
async function load(){if(!S.table)return;clearTimeout(S.timer);$('body').innerHTML='<tr class="loading"><td>Loading rows…</td></tr>';$('message').innerHTML='';let p=new URLSearchParams({table:S.table,page:S.page,size:S.size,q:S.q,dir:S.dir});if(S.sort)p.set('sort',S.sort);try{let r=await fetch('/api/rows?'+p);let d=await r.json();if(!r.ok)throw Error(d.error||'Could not read table');render(d)}catch(e){fail(e)}}
function render(d){$('title').textContent=nice(d.table);$('meta').textContent=`${d.total.toLocaleString()} ${d.total===1?'row':'rows'} · ${d.columns.length} columns`;$('head').innerHTML='<tr>'+d.columns.map(c=>`<th data-col="${esc(c.name)}">${esc(nice(c.name))}${S.sort===c.name?`<span class="sort">${S.dir==='asc'?'↑':'↓'}</span>`:''}</th>`).join('')+'</tr>';$('head').onclick=e=>{let th=e.target.closest('th');if(!th)return;let c=th.dataset.col;if(S.sort===c)S.dir=S.dir==='asc'?'desc':'asc';else{S.sort=c;S.dir='asc'}S.page=1;load()};if(!d.rows.length)$('body').innerHTML=`<tr><td colspan="${d.columns.length}"><div class="empty">No matching rows</div></td></tr>`;else $('body').innerHTML=d.rows.map(r=>'<tr>'+d.columns.map(c=>`<td class="${cellClass(c.name,c.type)}" title="${esc(r[c.name])}">${value(c.name,r[c.name])}</td>`).join('')+'</tr>').join('');let start=d.total?(d.page-1)*d.size+1:0,end=Math.min(d.page*d.size,d.total),pages=Math.max(1,Math.ceil(d.total/d.size));$('range').textContent=`Showing ${start.toLocaleString()}–${end.toLocaleString()} of ${d.total.toLocaleString()}`;$('page').textContent=`Page ${d.page} of ${pages}`;$('prev').disabled=d.page<=1;$('next').disabled=d.page>=pages}
function fail(e){$('message').innerHTML=`<div class="error">${esc(e.message)}</div>`;$('body').innerHTML=''}
$('search').oninput=e=>{S.q=e.target.value;S.page=1;clearTimeout(S.timer);S.timer=setTimeout(load,220)};$('pagesize').onchange=e=>{S.size=Number(e.target.value);S.page=1;load()};$('prev').onclick=()=>{if(S.page>1){S.page--;load()}};$('next').onclick=()=>{S.page++;load()};init();
</script></body></html>'''


def quote_ident(value: str) -> str:
    return '"' + value.replace('"', '""') + '"'


class App:
    def __init__(self, database: Path):
        self.database = database.resolve()

    def connect(self) -> sqlite3.Connection:
        connection = sqlite3.connect(f"file:{self.database.as_posix()}?mode=ro", uri=True)
        connection.row_factory = sqlite3.Row
        return connection

    def schema(self, connection: sqlite3.Connection) -> dict[str, list[dict[str, str]]]:
        names = [row[0] for row in connection.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        )]
        return {
            name: [{"name": row[1], "type": row[2] or ""}
                   for row in connection.execute(f"PRAGMA table_info({quote_ident(name)})")]
            for name in names
        }

    def info(self) -> dict:
        with self.connect() as db:
            tables = self.schema(db)
            return {"path": str(self.database), "tables": [
                {"name": name, "rows": db.execute(f"SELECT COUNT(*) FROM {quote_ident(name)}").fetchone()[0]}
                for name in tables
            ]}

    def rows(self, args: dict[str, list[str]]) -> dict:
        table = args.get("table", [""])[0]
        page = max(1, int(args.get("page", ["1"])[0]))
        size = min(250, max(1, int(args.get("size", ["50"])[0])))
        query = args.get("q", [""])[0].strip()
        sort = args.get("sort", [""])[0]
        direction = "ASC" if args.get("dir", ["desc"])[0].lower() == "asc" else "DESC"
        with self.connect() as db:
            schema = self.schema(db)
            if table not in schema:
                raise ValueError("Unknown table")
            columns = schema[table]
            names = [c["name"] for c in columns]
            if sort not in names:
                sort = next((c["name"] for c in columns if c["name"].lower() == "id"), names[0])
            params: list[object] = []
            where = ""
            if query:
                searchable = [c["name"] for c in columns]
                where = " WHERE " + " OR ".join(f"CAST({quote_ident(c)} AS TEXT) LIKE ?" for c in searchable)
                params = [f"%{query}%"] * len(searchable)
            source = quote_ident(table)
            total = db.execute(f"SELECT COUNT(*) FROM {source}{where}", params).fetchone()[0]
            sql = f"SELECT * FROM {source}{where} ORDER BY {quote_ident(sort)} {direction} LIMIT ? OFFSET ?"
            records = [dict(row) for row in db.execute(sql, [*params, size, (page - 1) * size])]
            return {"table": table, "columns": columns, "rows": records, "total": total, "page": page, "size": size}


def handler_for(app: App):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            url = urllib.parse.urlparse(self.path)
            try:
                if url.path == "/":
                    self.send(200, "text/html; charset=utf-8", HTML.encode())
                elif url.path == "/api/info":
                    self.json(200, app.info())
                elif url.path == "/api/rows":
                    self.json(200, app.rows(urllib.parse.parse_qs(url.query)))
                else:
                    self.json(404, {"error": "Not found"})
            except (ValueError, sqlite3.Error) as error:
                self.json(400, {"error": str(error)})

        def send(self, status: int, content_type: str, body: bytes):
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def json(self, status: int, value: object):
            self.send(status, "application/json; charset=utf-8", json.dumps(value).encode())

        def log_message(self, format, *args):
            pass

    return Handler


def main() -> None:
    parser = argparse.ArgumentParser(description="Browse a SQLite database in a local read-only UI")
    parser.add_argument("database", nargs="?", default=r"C:\Users\robert\Desktop\vicki\master.db")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--no-browser", action="store_true")
    args = parser.parse_args()
    database = Path(args.database)
    if not database.is_file():
        parser.error(f"database not found: {database}")
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler_for(App(database)))
    url = f"http://127.0.0.1:{args.port}"
    print(f"SQLite viewer: {url}\nDatabase: {database}\nPress Ctrl+C to stop.")
    if not args.no_browser:
        threading.Timer(0.35, lambda: webbrowser.open(url)).start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
