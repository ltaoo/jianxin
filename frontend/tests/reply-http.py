"""Send a Web reply through a loopback SMTP stub; never contact a real mailbox."""
import email
import base64
import http.client
import json
import pathlib
import socketserver
import sqlite3
import subprocess
import sys
import tempfile
import threading
import urllib.parse


class SMTP(socketserver.StreamRequestHandler):
    def handle(self):
        self.wfile.write(b'220 localhost SMTP\r\n')
        while line := self.rfile.readline():
            command = line.split()[0].upper()
            if command == b'EHLO':
                self.wfile.write(b'250-localhost\r\n250 AUTH PLAIN\r\n')
            elif command == b'AUTH':
                self.wfile.write(b'235 authenticated\r\n')
            elif command == b'DATA':
                self.wfile.write(b'354 continue\r\n')
                data = []
                while (line := self.rfile.readline()) not in (b'.\r\n', b''):
                    data.append(line)
                self.server.messages.append(email.message_from_bytes(b''.join(data)))
                self.wfile.write(b'250 delivered\r\n')
            elif command == b'QUIT':
                self.wfile.write(b'221 goodbye\r\n')
                return
            else:
                self.wfile.write(b'250 ok\r\n')


with socketserver.ThreadingTCPServer(('127.0.0.1', 0), SMTP) as smtp, tempfile.TemporaryDirectory(dir='/private/tmp') as work:
    smtp.messages = []
    threading.Thread(target=smtp.serve_forever, daemon=True).start()
    root = pathlib.Path(work)
    config = root / 'config.json'
    config.write_text(json.dumps({'general': {'data_dir': str(root / 'data'), 'compose': {'signature': 'Web signature'}},
        'accounts': [{'email': 'owner@example.com', 'auth': {'type': 'password', 'password': 'stub-password'},
            'imap': {'host': '127.0.0.1', 'port': 1, 'tls': 'none'},
            'smtp': {'host': '127.0.0.1', 'port': smtp.server_address[1], 'tls': 'none'}}]}))
    process = subprocess.Popen([str(pathlib.Path(sys.argv[1]).resolve()), '--config', str(config), 'web', '--port', '0'],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        address = urllib.parse.urlsplit(process.stdout.readline().strip())
        assert address.hostname == '127.0.0.1'
        def request(path, method='GET', body=None, headers=None):
            connection = http.client.HTTPConnection(address.hostname, address.port, timeout=10)
            connection.request(method, path, body=body, headers=headers or {})
            response = connection.getresponse()
            result = response.status, json.loads(response.read())
            connection.close()
            return result
        account_id = request('/api/mailboxes')[1]['accounts'][0]['id']
        db_path = root / 'data' / 'jianxin.sqlite'
        with sqlite3.connect(db_path) as db:
            db.execute("INSERT INTO folders(id,account_id,remote_id,name,kind,updated_at) VALUES('inbox',?,'INBOX','Inbox','inbox',0)", (account_id,))
            db.execute("INSERT INTO threads(id,account_id,subject,updated_at) VALUES('thread-1',?,'Meeting',0)", (account_id,))
            db.execute("INSERT INTO messages(id,account_id,folder_id,remote_id,thread_id,subject,subject_normalized,text_body,message_id_header,updated_at) VALUES('mail-1',?,'inbox','1','thread-1','Meeting','Meeting','Original','<parent@example.com>',0)", (account_id,))
            for role, address_value in [('from', 'sender@example.com'), ('reply_to', 'reply@example.com'), ('to', 'owner@example.com')]:
                db.execute('INSERT INTO addresses(message_id,role,position,email) VALUES(?,?,0,?)', ('mail-1', role, address_value))
        path = '/api/reply/mail-1'
        status, metadata = request('/api/message/mail-1/metadata')
        assert status == 200 and metadata['id'] == 'mail-1' and metadata['thread_id'] == 'thread-1'
        assert metadata['message_id_header'] == '<parent@example.com>' and metadata['remote_id'] == '1'
        assert metadata['from'][0]['email'] == 'sender@example.com'
        assert 'text_body' not in metadata and 'html_body' not in metadata
        with sqlite3.connect(db_path) as db:
            assert db.execute("SELECT is_read FROM messages WHERE id='mail-1'").fetchone()[0] == 0
        status, prepared = request(path)
        assert status == 200 and prepared['from'] == 'owner@example.com'
        assert prepared['to'][0]['email'] == 'reply@example.com' and prepared['subject'] == 'Re: Meeting'
        contacts = request('/api/contacts?q=sender')[1]['contacts']
        assert [c['email'] for c in contacts] == ['sender@example.com']
        payload = json.dumps({'to': [{'name': 'Edited recipient', 'email': 'edited@example.com'}],
            'cc': [{'name': None, 'email': 'copy@example.com'}], 'subject': prepared['subject'], 'body': 'Thanks from Web',
            'attachments': [{'filename': 'note.txt', 'data': base64.b64encode(b'attachment from Web').decode()}]})
        assert request(path, 'POST', payload)[0] == 403
        marker = {'X-Jianxin-Request': '1', 'Content-Type': 'application/json'}
        assert request(path, 'POST', payload, {**marker, 'Origin': 'https://evil.example'})[0] == 403
        assert request(path, 'POST', '{', marker)[0] == 400
        assert request(path, 'POST', payload, marker) == (200, {'ok': True})
        assert len(smtp.messages) == 1
        sent = smtp.messages[0]
        assert 'owner@example.com' in sent['From'] and 'edited@example.com' in sent['To']
        assert sent['Cc'] == 'copy@example.com'
        assert sent['In-Reply-To'] == '<parent@example.com>' and '<parent@example.com>' in sent['References']
        parts = list(sent.walk())
        text = next(p for p in parts if p.get_content_type() == 'text/plain' and p.get_filename() is None).get_payload(decode=True)
        assert b'Thanks from Web' in text and b'Web signature' in text
        attachment = next(p for p in parts if p.get_filename() == 'note.txt')
        assert attachment.get_payload(decode=True) == b'attachment from Web'
        with sqlite3.connect(db_path) as db:
            assert db.execute('SELECT status FROM outbox').fetchall() == [('sent',)]
            assert db.execute("SELECT thread_id FROM messages WHERE id != 'mail-1'").fetchall() == [('thread-1',)]
        smtp.shutdown()
        smtp.server_close()
        status, failure = request(path, 'POST', payload, marker)
        assert status == 502 and 'stub-password' not in failure['error']
        with sqlite3.connect(db_path) as db:
            assert db.execute("SELECT COUNT(*) FROM outbox WHERE status != 'sent'").fetchone()[0] == 1
            assert 'Thanks from Web' in db.execute("SELECT payload FROM outbox WHERE status != 'sent'").fetchone()[0]
        print('Web reply HTTP passed: protected POST, Reply-To, SMTP delivery, signature, threading and durable failure.')
    finally:
        process.terminate()
        process.wait(timeout=5)
        smtp.shutdown()
