"""Exercise the compiled loopback server with HTTP, without invoking a browser."""
import http.client
import json
import pathlib
import subprocess
import sys
import tempfile
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
        for name, content_type in [('app.js', 'text/javascript'), ('app.css', 'text/css')]:
            status, headers, body = request(prefix + 'assets/' + name)
            assert status == 200 and headers['Content-Type'].startswith(content_type)
            assert body == (pathlib.Path(__file__).resolve().parents[1] / 'dist' / name).read_bytes()
        status, _, body = request(prefix + 'api/mailboxes')
        assert status == 200 and json.loads(body) == {'accounts': [], 'counts': {'total': 0, 'unread': 0}}
        assert b'smoke-private-api-token' not in body
        status, _, body = request(prefix + 'api/threads')
        assert status == 200 and json.loads(body)['threads'] == []
        assert request(prefix + 'components')[0] == 200
        assert request(prefix + 'api/message/missing')[0] == 404
        assert request('/wrong/assets/app.js')[0] == 404
        assert request(prefix, headers={'Host': 'evil.example'})[0] == 404
        assert request(prefix + 'flags/missing?read=1', 'POST')[0] == 403
        assert request(prefix + 'flags/missing?read=1', 'POST', {'Origin': 'https://evil.example', 'X-Jianxin-Request': '1'})[0] == 403
        status, _, body = request(prefix + 'api/agent')
        assert status == 200 and json.loads(body)['configured'] is False
        assert json.loads(body)['messages'] == []
        assert request(prefix + 'api/agent/chat', 'POST', body='{"prompt":"hi"}')[0] == 403
        marker = {'X-Jianxin-Request': '1', 'Content-Type': 'application/json'}
        assert request(prefix + 'api/agent/chat', 'POST', marker, 'bad JSON')[0] == 400
        assert request(prefix + 'api/agent/chat', 'POST', marker, '{"prompt":"hi"}')[0] == 503
        assert request(prefix + 'api/agent?account=missing')[0] == 400
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
        print('HTTP smoke passed: shell, exact bundled assets, APIs, notification hook access/validation, gallery and idle exit. No browser or system notification invoked.')
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
