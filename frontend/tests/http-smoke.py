"""Exercise the compiled loopback server with HTTP, without invoking a browser."""
import http.client
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

executable = pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='jianxin-web-smoke-', dir='/private/tmp') as work:
    root = pathlib.Path(work)
    config = root / 'config.json'
    config.write_text(json.dumps({'general': {'data_dir': str(root / 'data'), 'api': {'token': 'smoke-private-api-token'}}, 'accounts': []}))
    process = subprocess.Popen([str(executable), '--config', str(config), 'web', '--port', '0', '--idle-seconds', '3'],
                               cwd=work, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        url = process.stdout.readline().strip()
        if not url.startswith('http://127.0.0.1:'):
            raise RuntimeError(process.stderr.read())
        address = urllib.parse.urlsplit(url)
        def request(path, method='GET', headers=None, body=None):
            connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
            connection.request(method, path, body=body, headers=headers or {})
            response = connection.getresponse()
            result = response.status, dict(response.getheaders()), response.read()
            connection.close()
            return result
        prefix = address.path
        assert prefix == "/"
        status, headers, body = request(prefix)
        assert status == 200 and b'/assets/app.js' in body
        assert "script-src 'self'" in headers['Content-Security-Policy']
        assert "img-src 'self'" in headers['Content-Security-Policy']
        for name, content_type in [('app.js', 'text/javascript'), ('app.css', 'text/css')]:
            status, headers, body = request(prefix + 'assets/' + name)
            assert status == 200 and headers['Content-Type'].startswith(content_type)
            assert body == (pathlib.Path(__file__).resolve().parents[1] / 'dist' / name).read_bytes()
        for platform in ['gmail', 'outlook', 'qq', 'foxmail', 'netease163', 'netease126', 'yeah', 'aliyun', 'sina', 'mail139', 'icloud', 'zoho', 'yahoo', 'fastmail', 'proton', 'tencent']:
            status, headers, body = request(prefix + 'assets/mail-providers/' + platform + '.svg')
            assert status == 200 and headers['Content-Type'].startswith('image/svg+xml')
            assert b'<svg' in body and b'<image' not in body
        assert request(prefix + 'assets/mail-providers/unknown.svg')[0] == 404
        status, _, body = request(prefix + 'api/mailboxes')
        assert status == 200 and json.loads(body) == {'accounts': [], 'counts': {'total': 0, 'unread': 0, 'starred': 0}}
        assert b'smoke-private-api-token' not in body
        status, _, body = request(prefix + 'api/threads')
        assert status == 200 and json.loads(body)['threads'] == []
        assert json.loads(request(prefix + 'api/threads?starred=1')[2])['threads'] == []
        assert request(prefix + 'components')[0] == 200
        assert request(prefix + 'api/message/missing')[0] == 404
        assert request('/wrong/assets/app.js')[0] == 404
        assert request(prefix, headers={'Host': 'evil.example'})[0] == 404
        assert request(prefix + 'flags/missing?read=1', 'POST')[0] == 403
        assert request(prefix + 'flags/missing?starred=1', 'POST')[0] == 403
        assert request(prefix + 'flags/missing?read=1', 'POST', {'Origin': 'https://evil.example', 'X-Jianxin-Request': '1'})[0] == 403
        status, _, body = request(prefix + 'api/agent')
        assert status == 200 and json.loads(body)['configured'] is False
        assert json.loads(body)['messages'] == []
        assert request(prefix + 'api/agent/chat', 'POST', body='{"prompt":"hi"}')[0] == 403
        marker = {'X-Jianxin-Request': '1', 'Content-Type': 'application/json'}
        assert request(prefix + 'api/agent/chat', 'POST', marker, 'bad JSON')[0] == 400
        assert request(prefix + 'api/agent/chat', 'POST', marker, '{"prompt":"hi"}')[0] == 503
        assert request(prefix + 'api/agent?account=missing')[0] == 400
        reload_path = prefix + 'api/mailboxes/reload'
        assert request(reload_path)[0] == 405
        assert request(reload_path, 'POST')[0] == 403
        assert request(reload_path, 'POST', {**marker, 'Origin': 'https://evil.example'})[0] == 403
        original = json.loads(config.read_text())
        updated = json.loads(config.read_text())
        updated['general']['data_dir'] = str(root / 'other-data')
        updated['general']['api']['token'] = 'ignored-until-restart'
        updated['general']['agent'] = {'model': 'ignored-until-restart'}
        updated['accounts'] = [{'id': 'added', 'email': 'added@example.com', 'platform': 'qq', 'auth': {'type': 'password', 'password': 'private-reload-password'}}]
        config.write_text(json.dumps(updated))
        assert json.loads(request(prefix + 'api/mailboxes')[2])['accounts'] == []
        status, _, body = request(reload_path, 'POST', marker)
        assert status == 200 and b'private-reload-password' not in body
        account = json.loads(body)['accounts'][0]
        assert account['email'] == 'added@example.com' and account['folders'] == []
        assert account['platform'] == 'qq' and account['provider'] == 'generic-imap'
        assert request(prefix + 'api/threads?account=' + account['id'])[0] == 200
        status, _, body = request(prefix + 'api/agent?account=' + account['id'])
        assert status == 200 and json.loads(body)['configured'] is False
        assert not (root / 'other-data').exists()
        updated['accounts'][0]['auth']['password'] = 'updated-private-password'
        updated['accounts'][0]['imap'] = {'host': 'localhost', 'port': 993, 'tls': 'tls'}
        config.write_text(json.dumps(updated))
        status, _, body = request(reload_path, 'POST', marker)
        assert status == 200 and b'updated-private-password' not in body
        assert json.loads(body)['accounts'][0]['id'] == account['id']
        for invalid in ['private-reload-password invalid JSON', json.dumps({'accounts': [{'email': 'invalid', 'auth': {'type': 'password', 'password': 'private-reload-password'}}]})]:
            config.write_text(invalid)
            status, _, body = request(reload_path, 'POST', marker)
            assert status == 400 and b'private-reload-password' not in body
            assert json.loads(request(prefix + 'api/mailboxes')[2])['accounts'][0]['id'] == account['id']
        config.write_text(json.dumps(original))
        assert json.loads(request(reload_path, 'POST', marker)[2])['accounts'] == []
        assert request(prefix + 'api/agent?account=' + account['id'])[0] == 400
        assert request(prefix + 'api/threads?account=' + account['id'])[0] == 404
        hook = '/api/notifications'
        auth = {'Authorization': 'Bearer smoke-private-api-token', 'Content-Type': 'application/json'}
        assert request(hook, headers=auth)[0] == 405
        assert request(hook, 'POST')[0] == 401
        assert request(hook, 'POST', {'X-Jianxin-Request': '1'})[0] == 401
        assert request(hook, 'POST', {**auth, 'Authorization': 'Bearer wrong-token'}, '{}')[0] == 401
        assert request(hook + '?token=smoke-private-api-token', 'POST', {'Content-Type': 'application/json'}, '{}')[0] == 401
        assert request(hook, 'POST', {**auth, 'Host': 'evil.example'}, '{}')[0] == 403
        assert request(hook, 'POST', {**auth, 'Origin': 'https://evil.example'}, '{}')[0] == 403
        assert request(prefix + 'api/notifications', 'POST', {**auth, 'X-Jianxin-Request': '1'}, '{}')[0] == 400
        assert request(prefix + 'flags/missing?read=1', 'POST', auth)[0] == 403
        assert request(hook, 'POST', {'Authorization': 'Bearer smoke-private-api-token'}, '{}')[0] == 415
        assert request(hook, 'POST', auth, '{')[0] == 400
        assert request(hook, 'POST', auth, '{}')[0] == 400
        assert request(hook, 'POST', auth, json.dumps({'title': 'test', 'body': 'x' * 4097}))[0] == 400
        assert request(hook, 'POST', auth, 'x' * 32769)[0] == 413
        process.wait(timeout=8)
        assert process.returncode == 0
        for api in [{}, {'token': ''}]:
            config.write_text(json.dumps({'general': {'data_dir': str(root / 'data'), 'api': api}, 'accounts': []}))
            process = subprocess.Popen([str(executable), '--config', str(config), 'web', '--port', '0', '--idle-seconds', '1'], cwd=work, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            disabled = urllib.parse.urlsplit(process.stdout.readline().strip())
            connection = http.client.HTTPConnection(disabled.hostname, disabled.port, timeout=5)
            connection.request('POST', '/api/notifications', body='{}', headers=auth)
            response = connection.getresponse()
            assert response.status == 401
            response.read(); connection.close()
            process.wait(timeout=5)
            assert process.returncode == 0
        if sys.platform == 'darwin':
            # Exercise launchd restart through PATH, without touching the user's service.
            state_path = root / 'daemon-state.json'
            state = {'config': str(config), 'pid': 12345, 'calls': [], 'fail': False}
            state_path.write_text(json.dumps(state))
            bin_dir = root / 'bin'; bin_dir.mkdir()
            launchctl = bin_dir / 'launchctl'
            launchctl.write_text(f'''#!{sys.executable}
import json, pathlib, sys
path = pathlib.Path({str(state_path)!r})
state = json.loads(path.read_text())
if sys.argv[1] == 'print':
    print('arguments = {{\\n/bin/jianxin\\n--config\\n' + state['config'] + '\\nwatch\\n--notify\\n--interval\\n60\\n}}\\npid = ' + str(state['pid']))
elif sys.argv[1] == 'kickstart':
    assert sys.argv[2:] == ['-k', 'gui/' + str(__import__('os').getuid()) + '/com.jianxin.mail-daemon']
    state['calls'].append(sys.argv[1:])
    if not state['fail']: state['pid'] += 1
    path.write_text(json.dumps(state))
    if state['fail']:
        print('private-daemon-error', file=sys.stderr)
        sys.exit(1)
else:
    raise AssertionError(sys.argv)
''')
            launchctl.chmod(0o755)
            config.write_text(json.dumps(original))
            process = subprocess.Popen([str(executable), '--config', str(config), 'web', '--port', '0', '--idle-seconds', '3'], cwd=work, env={**os.environ, 'PATH': str(bin_dir) + os.pathsep + os.environ['PATH']}, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            address = urllib.parse.urlsplit(process.stdout.readline().strip())
            status, _, body = request(reload_path, 'POST', marker)
            assert status == 200 and json.loads(body)['daemon']['status'] == 'restarted'
            state = json.loads(state_path.read_text()); assert state['pid'] == 12346 and len(state['calls']) == 1
            config.write_text('{')
            assert request(reload_path, 'POST', marker)[0] == 400
            assert len(json.loads(state_path.read_text())['calls']) == 1
            config.write_text(json.dumps(original))
            other_config = root / 'other-config.json'; other_config.write_text(json.dumps(original))
            state['config'] = str(other_config); state_path.write_text(json.dumps(state))
            assert json.loads(request(reload_path, 'POST', marker)[2])['daemon']['status'] == 'not_running'
            assert len(json.loads(state_path.read_text())['calls']) == 1
            state['config'] = str(config); state['fail'] = True; state_path.write_text(json.dumps(state))
            status, _, body = request(reload_path, 'POST', marker)
            assert status == 200 and json.loads(body)['daemon']['status'] == 'failed'
            assert b'private-daemon-error' not in body and json.loads(body)['accounts'] == []
            state = json.loads(state_path.read_text()); state['fail'] = False; state_path.write_text(json.dumps(state))
            assert json.loads(request(reload_path, 'POST', marker)[2])['daemon']['status'] == 'restarted'
            assert len(json.loads(state_path.read_text())['calls']) == 3
            changed_directory = json.loads(json.dumps(original))
            changed_directory['general']['data_dir'] = str(root / 'changed-data')
            config.write_text(json.dumps(changed_directory))
            status, _, body = request(reload_path, 'POST', marker)
            assert status == 200 and json.loads(body)['daemon']['status'] == 'failed'
            assert '数据目录已改变' in json.loads(body)['daemon']['error']
            assert len(json.loads(state_path.read_text())['calls']) == 3 and not (root / 'changed-data').exists()
            process.wait(timeout=8); assert process.returncode == 0
        print('HTTP smoke passed: mailbox reload and daemon restart, bundled assets, APIs, notification hook, gallery and idle exit. No real daemon or notification invoked.')
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)

    # OAuth protocol is exercised against a loopback mock, never a real mailbox.
    oauth = {'authorized': False, 'mode': 'ok', 'starts': 0, 'polls': 0}
    class OAuthHandler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_POST(self):
            form = urllib.parse.parse_qs(self.rfile.read(int(self.headers['Content-Length'])).decode())
            assert form['client_id'] == ['fixture-client']
            if self.path.endswith('/devicecode'):
                oauth['starts'] += 1
                result = {'user_code': 'ABCD-EFGH', 'device_code': 'private-device-code', 'verification_uri': 'javascript:alert(1)' if oauth['mode'] == 'unsafe' else 'https://login.example.test/device', 'expires_in': 12, 'interval': 1}
                status = 200
            else:
                oauth['polls'] += 1
                assert form['device_code'] == ['private-device-code']
                if oauth['mode'] == 'denied':
                    status, result = 400, {'error': 'authorization_declined', 'error_description': 'private-provider-error'}
                elif not oauth['authorized']:
                    status, result = 400, {'error': 'authorization_pending'}
                else:
                    status, result = 200, {'access_token': 'private-access-token', 'refresh_token': 'private-refresh-token', 'expires_in': 3600}
            body = json.dumps(result).encode()
            self.send_response(status); self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    provider = http.server.ThreadingHTTPServer(('127.0.0.1', 0), OAuthHandler)
    thread = threading.Thread(target=provider.serve_forever, daemon=True); thread.start()
    oauth_config = {'general': {'data_dir': str(root / 'oauth-data')}, 'accounts': [
        {'email': 'oauth@example.com', 'platform': 'outlook', 'auth': {'type': 'oauth2', 'client_id': 'fixture-client', 'token_endpoint': f'http://127.0.0.1:{provider.server_port}/oauth/token'}},
        {'email': 'password@example.com', 'auth': {'type': 'password', 'password': 'fixture-password'}},
    ]}
    config.write_text(json.dumps(oauth_config))
    # Disable daemon detection in the test rather than inspecting the user's service.
    stub_dir = root / 'oauth-bin'; stub_dir.mkdir(); stub = stub_dir / 'launchctl'; stub.write_text('#!/bin/sh\nexit 1\n'); stub.chmod(0o755)
    process = subprocess.Popen([str(executable), '--config', str(config), 'web', '--port', '0', '--idle-seconds', '0'], cwd=work,
        env={**os.environ, 'PATH': str(stub_dir) + os.pathsep + os.environ['PATH']}, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        url = process.stdout.readline().strip(); assert url.startswith('http://127.0.0.1:')
        address = urllib.parse.urlsplit(url)
        accounts = json.loads(request('/api/mailboxes')[2])['accounts']; account = accounts[0]; password = accounts[1]
        login_path = '/api/mailboxes/oauth?account=' + account['id']; marker = {'X-Jianxin-Request': '1'}
        def login_state():
            status, _, body = request(login_path); assert status == 200
            for secret in [b'private-device-code', b'private-access-token', b'private-refresh-token', b'private-provider-error']:
                assert secret not in body
            return json.loads(body)
        def wait_state(expected):
            deadline = time.monotonic() + 6
            while time.monotonic() < deadline:
                value = login_state()
                if value['state'] == expected: return value
                time.sleep(.05)
            raise AssertionError(login_state())
        assert request(login_path, 'POST')[0] == 403
        assert request(login_path, 'POST', {**marker, 'Origin': 'https://evil.example'})[0] == 403
        assert request('/api/mailboxes/oauth?account=missing', 'POST', marker)[0] == 400
        assert request('/api/mailboxes/oauth?account=' + password['id'], 'POST', marker)[0] == 400
        assert oauth['starts'] == 0 and login_state()['state'] == 'idle'
        status_dir = root / 'oauth-data' / 'account-status'; status_dir.mkdir(); status_file = status_dir / (account['id'] + '.txt'); status_file.write_text('等待 OAuth 登录')
        assert request(login_path, 'POST', marker)[0] == 200
        pending = wait_state('pending'); assert pending['user_code'] == 'ABCD-EFGH' and pending['verification_uri'].startswith('https://')
        assert request(login_path, 'POST', marker)[0] == 200 and oauth['starts'] == 1
        assert request('/api/mailboxes/reload', 'POST', marker)[0] == 409
        oauth['authorized'] = True; wait_state('complete')
        saved = json.loads(config.read_text()); assert saved['accounts'][0]['auth']['refresh_token'] == 'private-refresh-token'
        assert saved['accounts'][0]['platform'] == 'outlook' and not status_file.exists()
        status, _, body = request('/api/mailboxes/reload', 'POST', marker); assert status == 200
        assert b'private-refresh-token' not in body and json.loads(body)['accounts'][0]['status']['state'] == 'not_synced'
        oauth['mode'] = 'denied'; assert request(login_path, 'POST', marker)[0] == 200
        assert wait_state('failed')['busy'] is False
        polls = oauth['polls']; oauth['mode'] = 'unsafe'; assert request(login_path, 'POST', marker)[0] == 200
        assert wait_state('failed')['busy'] is False and oauth['polls'] == polls
        oauth['mode'] = 'ok'; oauth['authorized'] = False; assert request(login_path, 'POST', marker)[0] == 200; wait_state('pending')
        config.write_text('{'); oauth['authorized'] = True
        assert '保存失败' in wait_state('failed')['error'] and config.read_text() == '{'
        print('OAuth smoke passed: explicit start, protected writes, duplicate prevention, token persistence, reload, denial, unsafe URL rejection and save failure. No real OAuth login.')
    finally:
        process.terminate(); process.wait(timeout=5); provider.shutdown(); provider.server_close()
