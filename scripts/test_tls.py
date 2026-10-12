#!/usr/bin/env python3
"""Verify real CFNetwork TLS and STARTTLS reject an untrusted local server."""
import socket
import ssl
import subprocess
import tempfile
import threading
from pathlib import Path
root=Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='jianxin-tls-') as temp:
    temp=Path(temp)
    subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(temp/'key.pem'),'-out',str(temp/'cert.pem'),'-days','1','-subj','/CN=localhost'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    probe=temp/'probe'
    sources=[root/'ios/Jianxin'/f for f in ['Models.swift','MIME.swift','TLSSocket.swift','IMAPClient.swift','ConfigEncryption.swift','RemoteConfig.swift']]
    subprocess.run(['swiftc','-module-cache-path',str(temp/'cache'),*[str(p) for p in sources],str(root/'tests/mobile/swift/TLSRejectionProbe.swift'),'-o',str(probe)],check=True)
    context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.minimum_version=ssl.TLSVersion.TLSv1_2;context.load_cert_chain(temp/'cert.pem',temp/'key.pem')
    for mode in ['tls','starttls','https']:
        errors=[]
        server=socket.socket();server.bind(('127.0.0.1',0));server.listen(1);server.settimeout(15)
        port=server.getsockname()[1]
        def handle():
            try:
                with server.accept()[0] as connection:
                    connection.settimeout(15)
                    if mode=='starttls':
                        connection.sendall(b'* OK test server\r\n')
                        line=b''
                        while not line.endswith(b'\r\n'):line += connection.recv(1)
                        if b'STARTTLS' not in line:raise AssertionError('credentials sent before STARTTLS')
                        tag=line.split()[0];connection.sendall(tag+b' OK upgrade\r\n')
                    first=connection.recv(5,socket.MSG_PEEK)
                    if not first or first[0]!=0x16:raise AssertionError(f'non-TLS bytes after upgrade: {first!r}')
                    try:
                        with context.wrap_socket(connection,server_side=True) as tls:
                            raise AssertionError('untrusted certificate unexpectedly accepted')
                    except ssl.SSLError:pass
            except Exception as error:errors.append(error)
        thread=threading.Thread(target=handle);thread.start()
        result=subprocess.run([str(probe),str(port),mode],capture_output=True,text=True,timeout=30)
        thread.join(20);server.close()
        if thread.is_alive() or errors or result.returncode:
            raise RuntimeError(f'{mode} failed: {errors}, {result.stdout}, {result.stderr}')
        print(f'{mode}: real TLS handshake, untrusted certificate rejected, no plaintext credentials')
