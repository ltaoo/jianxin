"""Real CLI sync -> persistent history -> Web API, using only local mock IMAP."""
import http.client
import json
import pathlib
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import urllib.parse

executable = pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='jianxin-sync-history-') as work:
    root = pathlib.Path(work)
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0))
    listener.listen(1)
    listener.settimeout(10)
    port = listener.getsockname()[1]

    def imap():
        with listener:
            stream, _ = listener.accept()
            with stream:
                stream.settimeout(5)
                stream.sendall(b'* OK IMAP4rev1 ready\r\n')
                with stream.makefile('rb') as reader:
                    for line in reader:
                        tag, command, *_ = line.split()
                        if command.upper() == b'CAPABILITY':
                            stream.sendall(b'* CAPABILITY IMAP4rev1\r\n')
                        if command.upper() == b'LOGOUT':
                            stream.sendall(b'* BYE\r\n' + tag + b' OK done\r\n')
                            return
                        stream.sendall(tag + b' OK done\r\n')

    thread = threading.Thread(target=imap, daemon=True)
    thread.start()
    config_path = root / 'config.json'
    config = {'general': {'data_dir': str(root / 'data')}, 'accounts': [
        {'email': 'sync@example.com', 'auth': {'type': 'password', 'password': 'private-password'},
         'imap': {'host': '127.0.0.1', 'port': port, 'tls': 'none'},
         'smtp': {'host': 'localhost', 'port': 465}},
        {'email': 'pending@outlook.com', 'auth': {'type': 'oauth2'}}]}
    config_path.write_text(json.dumps(config))
    command = [str(executable), '--config', str(config_path), '-a', 'sync@example.com', 'sync']
    first = subprocess.run(command, cwd=work, capture_output=True, timeout=10)
    assert first.returncode == 0, first.stderr.decode()
    thread.join(timeout=5)
    assert not thread.is_alive()
    with socket.socket() as refused:
        refused.bind(('127.0.0.1', 0))
        config['accounts'][0]['imap']['port'] = refused.getsockname()[1]
        config_path.write_text(json.dumps(config))
        subprocess.run(command, cwd=work, capture_output=True, timeout=10)
        with sqlite3.connect(root / 'data' / 'jianxin.sqlite') as db:
            rows = db.execute('SELECT completed_at,status FROM sync_logs ORDER BY id').fetchall()
            assert [row[1] for row in rows] == ['synced', 'sync_failed']
            assert all(row[0] > 0 for row in rows)

        process = subprocess.Popen([str(executable), '--config', str(config_path), 'web', '--port', '0'],
                                   cwd=work, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            address = urllib.parse.urlsplit(process.stdout.readline().strip())
            assert address.hostname == '127.0.0.1'

            def request(path):
                connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
                connection.request('GET', path)
                response = connection.getresponse()
                body = response.read()
                connection.close()
                assert b'private-password' not in body
                return response.status, body

            assert request('/logs')[0] == 200
            status, body = request('/api/sync-logs')
            assert status == 200
            result = json.loads(body)
            assert len(result['logs']) == 2
            assert result['logs'][0]['result']['state'] == 'sync_failed'
            synced = next(account for account in result['accounts'] if account['email'] == 'sync@example.com')
            pending = next(account for account in result['accounts'] if account['email'] == 'pending@outlook.com')
            assert synced['last_sync_at'] == rows[-1][0]
            assert pending['last_sync_at'] is None
            assert pending['status']['state'] == 'oauth_required'
            assert json.loads(request('/api/sync-logs?account=' + pending['id'])[1])['logs'] == []
            subprocess.run(command, cwd=work, capture_output=True, timeout=10)
            assert len(json.loads(request('/api/sync-logs')[1])['logs']) == 3
            assert request('/api/sync-logs?account=unknown')[0] == 404
        finally:
            process.terminate()
            process.wait(timeout=5)
print('CLI sync success/failure, persisted timestamps, live Web logs and mailbox filtering passed.')
