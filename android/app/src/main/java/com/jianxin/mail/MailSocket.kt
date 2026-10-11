package com.jianxin.mail

import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.net.InetSocketAddress
import java.net.Socket
import javax.net.ssl.SSLSocket
import javax.net.ssl.SSLSocketFactory

interface MailWire : Closeable {
    fun line(value: String)
    fun write(data: ByteArray)
    fun readLine(): String
    fun readBytes(count: Int): ByteArray
    fun startTLS()
}
class MailSocket(private val host: String, port: Int, implicitTLS: Boolean) : MailWire {
    private val transport = Socket()
    @Volatile private var socket = transport
    private val cancellation = MailCancellation.current()
    private lateinit var input: BufferedInputStream
    private lateinit var output: BufferedOutputStream
    init {
        cancellation?.attach(transport)
        try {
            socket.connect(InetSocketAddress(host, port), 15000); socket.soTimeout = 60000
            input = BufferedInputStream(socket.getInputStream()); output = BufferedOutputStream(socket.getOutputStream())
            if (implicitTLS) startTLS()
            cancellation?.check()
        } catch (e: Exception) { close(); throw e }
    }
    override fun startTLS() {
        val tls = (SSLSocketFactory.getDefault() as SSLSocketFactory).createSocket(socket, host, socket.port, true) as SSLSocket
        tls.soTimeout = 60000
        tls.enabledProtocols = tls.supportedProtocols.filter { it == "TLSv1.2" || it == "TLSv1.3" }.toTypedArray()
        tls.sslParameters = tls.sslParameters.apply { endpointIdentificationAlgorithm = "HTTPS" }
        try { tls.startHandshake() } catch (e: Exception) { tls.close(); throw MailException("安全连接失败，检查服务器地址与证书") }
        socket = tls; input = BufferedInputStream(tls.inputStream); output = BufferedOutputStream(tls.outputStream)
    }
    override fun line(value: String) = write((value + "\r\n").toByteArray())
    override fun write(data: ByteArray) { cancellation?.check(); output.write(data); output.flush() }
    override fun readLine(): String {
        cancellation?.check()
        val out = ByteArrayOutputStream()
        var last = -1
        while (out.size() <= 256 * 1024) {
            val c = input.read(); if (c < 0) throw MailException("邮箱连接已断开")
            if (c == 10 && last == 13) return out.toByteArray().dropLast(1).toByteArray().toString(Charsets.UTF_8)
            out.write(c); last = c
        }
        throw MailException("邮件服务器响应过长")
    }
    override fun readBytes(count: Int): ByteArray {
        cancellation?.check()
        if (count !in 0..MAX_MESSAGE) throw MailException("邮件超过 25 MB，使用桌面客户端查看")
        val bytes = ByteArray(count); var offset = 0
        while (offset < count) { cancellation?.check(); val n = input.read(bytes, offset, count - offset); if (n <= 0) throw MailException("邮箱连接已断开"); offset += n }
        return bytes
    }
    override fun close() { try { socket.close() } catch (_: Exception) { } finally { cancellation?.detach(transport) } }
    companion object { const val MAX_MESSAGE = 25 * 1024 * 1024 }
}
