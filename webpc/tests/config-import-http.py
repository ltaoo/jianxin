"""Use the real compiled server; never launch or automate a browser."""
import http.client
import http.server
import json
import pathlib
import socket
import subprocess
import sys
import tempfile
import time
import threading
import urllib.parse

executable = pathlib.Path(sys.argv[1]).resolve()
fixture = json.loads((pathlib.Path(__file__).resolve().parents[2] / 'tests/mobile/fixtures/config.json').read_text(encoding='utf-8'))
incoming = json.loads(fixture['plaintext'])
with tempfile.TemporaryDirectory(prefix='jianxin-config-import-') as directory:
    root = pathlib.Path(directory)
    config = root / 'config.json'
    original = {'general': {'data_dir': str(root / 'data'), 'sync': {'timeout_secs': 7}}, 'accounts': []}
    config.write_text(json.dumps(original), encoding='utf-8')
    process = subprocess.Popen([str(executable), '--config', str(config), 'web', '--port', '0', '--import-config'],
                               cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        address = urllib.parse.urlsplit(process.stdout.readline().strip())
        assert address.fragment == 'config-import'
        def request(path, value=None, origin=None, marker=True):
            connection = http.client.HTTPConnection(address.hostname, address.port, timeout=10)
            headers = {'Content-Type': 'application/json'}
            if marker: headers['X-Jianxin-Request'] = '1'
            if origin: headers['Origin'] = origin
            connection.request('POST', '/api/config/' + path, body=json.dumps(value or {}), headers=headers)
            response = connection.getresponse()
            result = response.status, json.loads(response.read())
            connection.close()
            return result
        assert request('preview', incoming, marker=False)[0] == 403
        assert request('import', incoming, origin='https://evil.example')[0] == 403
        status, preview = request('preview', incoming)
        assert status == 200 and len(preview['accounts']) == 2
        assert 'sample-password' not in json.dumps(preview)
        assert json.loads(config.read_text()) == original
        for url in ['http://example.com/a', 'https://user:pass@example.com/a', 'https://example.com/a#key']:
            assert request('source', {'url': url})[0] == 400
        selected = {'accounts': [incoming['accounts'][0]]}
        for attempt in range(20):
            status, result = request('import', selected)
            if status != 409: break
            time.sleep(.05)
        assert status == 200 and result['imported'] == 1, result
        saved = json.loads(config.read_text())
        assert saved['accounts'][0]['email'] == 'sample@qq.com'
        assert saved['general']['data_dir'] == original['general']['data_dir']
        assert saved['general']['sync']['timeout_secs'] == 7
        # Merge a second account and update one existing account without duplication.
        for accounts in [[incoming['accounts'][1]], incoming['accounts']]:
            assert request('import', {'accounts': accounts})[0] == 200
        saved = json.loads(config.read_text())
        assert len(saved['accounts']) == 2
        before = config.read_bytes()
        assert request('import', {'accounts': [{'email': 'invalid'}]})[0] == 400
        assert config.read_bytes() == before
        assert not list(root.glob('*.tmp'))
        connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
        connection.request('GET', '/api/mailboxes')
        accounts = json.loads(connection.getresponse().read())['accounts']
        assert len(accounts) == 2
        connection.close()
        # The manual form gets the same presets, verifies before saving, and uses the import merger.
        connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
        connection.request('GET', '/api/config/presets')
        response = connection.getresponse()
        assert response.status == 200
        presets = json.loads(response.read())['providers']
        connection.close()
        assert len(presets) == 7
        outlook = next(row for row in presets if row['key'] == 'outlook')
        manual = {'accounts': [{'email': 'manual@outlook.com', 'display_name': '手动添加', 'platform': outlook['platform'],
            'provider': outlook['provider'], 'auth': {'type': 'oauth2'}, 'imap': outlook['imap'], 'smtp': outlook['smtp']}]}
        before = config.read_bytes()
        assert request('preview', {**manual, 'verify_connection': True}, marker=False)[0] == 403
        status, result = request('preview', {**manual, 'verify_connection': True})
        assert status == 200 and result['accounts'][0]['needs_authorization']
        assert not result['accounts'][0]['connection_verified'] and config.read_bytes() == before
        with socket.socket() as closed:
            closed.bind(('127.0.0.1', 0))
            failed = {'verify_connection': True, 'accounts': [{'email': 'failed@example.com', 'auth': {'type': 'app_password', 'password': 'private-manual-password'},
                'imap': {'host': '127.0.0.1', 'port': closed.getsockname()[1], 'tls': 'tls'}, 'smtp': {'host': 'smtp.example.com', 'port': 465}}]}
            status, error = request('preview', failed)
            assert status == 400 and 'private-manual-password' not in json.dumps(error)
        assert config.read_bytes() == before
        for attempt in range(20):
            status, result = request('import', manual)
            if status != 409: break
            time.sleep(.05)
        assert status == 200, result
        saved = json.loads(config.read_text())
        assert len(saved['accounts']) == 3
        assert saved['general']['data_dir'] == original['general']['data_dir']
        assert saved['general']['sync']['timeout_secs'] == original['general']['sync']['timeout_secs']
        assert request('preview', manual)[1]['accounts'][0]['existing']
        assert config.stat().st_mode & 0o077 == 0
        # OAuth-only accounts stay in the preview and config before any login.
        pending = {'accounts': [{'email': 'pending@outlook.com', 'auth': {'type': 'oauth2'}}]}
        status, preview = request('preview', pending)
        assert status == 200 and preview['accounts'][0]['needs_authorization']
        for attempt in range(20):
            status, result = request('import', pending)
            if status != 409: break
            time.sleep(.05)
        assert status == 200, result
        saved = json.loads(config.read_text())
        oauth = next(account for account in saved['accounts'] if account['email'] == 'pending@outlook.com')['auth']
        assert oauth['token_endpoint'] == 'https://login.microsoftonline.com/consumers/oauth2/v2.0/token'
        assert 'offline_access' in oauth['scopes'] and 'refresh_token' not in oauth
        def mailboxes():
            connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
            connection.request('GET', '/api/mailboxes')
            result = json.loads(connection.getresponse().read())['accounts']
            connection.close()
            return result
        mailbox = next(account for account in mailboxes() if account['email'] == 'pending@outlook.com')
        assert mailbox['status']['state'] == 'oauth_required', mailbox['status']
        assert mailbox['oauth_config']['needs_setup']
        before = config.read_bytes()
        def settings(value):
            connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
            connection.request('POST', '/api/mailboxes/oauth-settings', body=json.dumps(value),
                               headers={'Content-Type': 'application/json', 'X-Jianxin-Request': '1'})
            response = connection.getresponse()
            result = response.status, json.loads(response.read())
            connection.close()
            return result
        update = {'account': mailbox['id'], 'client_id': '', 'token_endpoint': oauth['token_endpoint']}
        assert settings(update)[0] == 400 and config.read_bytes() == before
        update['client_id'] = 'fixture-client-id'
        assert settings(update)[0] == 200
        updated = next(account for account in mailboxes() if account['id'] == mailbox['id'])
        assert not updated['oauth_config']['needs_setup']
        assert updated['status']['state'] == 'oauth_required'
        # Saving application settings never automatically starts a device login.
        connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
        connection.request('GET', '/api/mailboxes/oauth?account=' + mailbox['id'])
        login = json.loads(connection.getresponse().read())
        assert login['state'] == 'idle' and not login['busy']
        connection.close()
        # Exercise explicit device-code login with a local fake provider.
        calls = []
        class Provider(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                calls.append(self.path)
                self.rfile.read(int(self.headers.get('Content-Length', '0')))
                payload = ({'user_code': 'TEST-CODE', 'device_code': 'fixture-device-code',
                            'verification_uri': 'https://microsoft.com/devicelogin', 'expires_in': 30, 'interval': 1}
                           if self.path.endswith('/devicecode') else
                           {'access_token': 'fixture-access-token', 'refresh_token': 'fixture-refresh-token', 'expires_in': 3600})
                body = json.dumps(payload).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *_): pass
        provider = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        provider_thread = threading.Thread(target=provider.serve_forever, daemon=True)
        provider_thread.start()
        try:
            mock = {'accounts': [{'email': 'device-login@outlook.com', 'auth': {'type': 'oauth2', 'client_id': 'fixture-app',
                                'token_endpoint': 'http://127.0.0.1:%d/token' % provider.server_port}}]}
            assert request('import', mock)[0] == 200
            device_account = next(account for account in mailboxes() if account['email'] == 'device-login@outlook.com')
            assert device_account['status']['state'] == 'oauth_required' and not calls
            def device_login(method):
                connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
                connection.request(method, '/api/mailboxes/oauth?account=' + device_account['id'],
                                   headers={'X-Jianxin-Request': '1'})
                response = connection.getresponse()
                result = response.status, json.loads(response.read())
                connection.close()
                return result
            assert device_login('POST')[0] == 200
            saw_prompt = False
            for attempt in range(100):
                status, login = device_login('GET')
                assert status == 200
                saw_prompt |= login.get('user_code') == 'TEST-CODE'
                if login['state'] in ('complete', 'failed'): break
                time.sleep(.05)
            assert saw_prompt and login['state'] == 'complete', login
            saved = json.loads(config.read_text())
            auth = next(account for account in saved['accounts'] if account['email'] == 'device-login@outlook.com')['auth']
            assert auth['refresh_token'] == 'fixture-refresh-token'
            connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
            connection.request('POST', '/api/mailboxes/reload', headers={'X-Jianxin-Request': '1'})
            response = connection.getresponse()
            assert response.status == 200
            response.read()
            connection.close()
            authorized = next(account for account in mailboxes() if account['id'] == device_account['id'])
            assert authorized['status']['state'] != 'oauth_required'
            assert calls == ['/devicecode', '/token']
        finally:
            provider.shutdown()
            provider.server_close()
            provider_thread.join(timeout=3)
        print('HTTP passed: presets, manual validation and save, import, pending OAuth, application settings, explicit device login, refresh-token persistence and live status updates.')
    finally:
        process.terminate()
        process.wait(timeout=10)
