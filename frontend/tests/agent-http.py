#!/usr/bin/env python3
"""Async agent HTTP integration using a loopback model and empty mail fixtures."""
import http.server, threading, tempfile, pathlib, subprocess, json, re, time, sys, urllib.request, urllib.error, os

requests = []
model_started = threading.Event()
resume_model = threading.Event()
class Model(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_POST(self):
        payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        requests.append(payload)
        if len(requests) == 1:
            model_started.set()
            assert resume_model.wait(5)
        assert payload['stream'] is True
        self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
        if payload['messages'][-1]['role'] == 'user':
            delta = {'tool_calls': [{'index': 0, 'id': 'accounts-1', 'function': {'name': 'mail_list_accounts', 'arguments': '{}'}}]}
        else:
            assert payload['messages'][-1]['role'] == 'tool'
            delta = {'content': '当前未配置邮箱。'}
        self.wfile.write(('data: ' + json.dumps({'choices': [{'delta': delta}]}) + '\n\ndata: [DONE]\n\n').encode()); self.wfile.flush()

model = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Model)
threading.Thread(target=model.serve_forever, daemon=True).start()
with tempfile.TemporaryDirectory(prefix='jianxin-agent-http-', dir='/private/tmp' if sys.platform == 'darwin' else None) as directory:
    directory = pathlib.Path(directory)
    config = directory / 'config.json'
    config.write_text(json.dumps({'general': {'data_dir': str(directory / 'data'), 'agent': {'base_url': f'http://127.0.0.1:{model.server_port}/v1', 'model': 'mock-model'}}, 'accounts': []}))
    def launch():
        log = open(directory / 'web.log', 'w+')
        process = subprocess.Popen([sys.argv[1], '--config', str(config), 'web', '--port', '0', '--idle-seconds', '0'], stdout=log, stderr=log)
        for _ in range(100):
            log.seek(0); match = re.search(r'http://127\.0\.0\.1:\d+/', log.read())
            if match: return process, log, match.group()
            if process.poll() is not None: raise RuntimeError('fixture Web service failed')
            time.sleep(.05)
        raise RuntimeError('fixture Web service start timeout')
    process, log, root = launch()
    def call(path, payload=None, marker=True):
        data = None if payload is None else json.dumps(payload).encode()
        headers = {'Content-Type': 'application/json'}
        if data and marker: headers['X-Jianxin-Request'] = '1'
        req = urllib.request.Request(root + 'api/agent' + path, data=data, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=5) as response: return response.status, json.load(response)
        except urllib.error.HTTPError as error: return error.code, json.load(error)
    try:
        assert call('')[1]['configured'] is True
        assert requests == []  # Opening history never calls the model.
        assert call('/chat', {'prompt': '列出邮箱'}, marker=False)[0] == 403
        assert call('/chat', {'prompt': '列出邮箱'})[0] == 202
        assert model_started.wait(5)
        reload_request = urllib.request.Request(root + 'api/mailboxes/reload', data=b'{}', headers={'X-Jianxin-Request': '1'})
        try:
            urllib.request.urlopen(reload_request, timeout=5)
            raise AssertionError('reload accepted while the agent was busy')
        except urllib.error.HTTPError as error:
            assert error.code == 409
        resume_model.set()
        for _ in range(100):
            state = call('')[1]
            if not state['busy']: break
            time.sleep(.03)
        assert not state['busy'] and not state['error'], state
        assert state['messages'][-1]['content'] == '当前未配置邮箱。'
        assert state['trace'] == ['mail_list_accounts · 完成'] and state['drafts'] == []
        assert len(requests) == 2
        history = next((directory / 'data' / 'agent-chats').glob('*.json'))
        if os.name == 'posix': assert history.stat().st_mode & 0o777 == 0o600
        process.terminate(); process.wait(timeout=5); log.close()
        process, log, root = launch()
        assert call('')[1]['messages'][-1]['content'] == '当前未配置邮箱。'
        assert call('/clear', {})[0] == 200
        assert call('')[1]['messages'] == []
        assert json.loads(history.read_text())['messages'] == []
        print('Agent HTTP passed: protected POST, explicit model invocation, streaming tool loop, private persisted history, restart and clear. No browser or real mail used.')
    finally:
        resume_model.set()
        process.terminate(); process.wait(timeout=5); log.close(); model.shutdown(); model.server_close()
