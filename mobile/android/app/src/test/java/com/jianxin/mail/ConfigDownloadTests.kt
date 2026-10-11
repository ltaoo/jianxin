package com.jianxin.mail

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.net.URL
import java.io.ByteArrayInputStream
import java.security.cert.Certificate
import javax.net.ssl.HttpsURLConnection

private class ConfigConnection(url: URL, private val data: ByteArray, private val code: Int = 200, private val length: Long = -1, private val location: String? = null): HttpsURLConnection(url) {
    var closed = false
    var read = false
    override fun connect() { }
    override fun disconnect() { closed = true }
    override fun usingProxy() = false
    override fun getResponseCode() = code
    override fun getContentLengthLong() = length
    override fun getHeaderField(name: String): String? = if (name.equals("Location",true)) location else null
    override fun getInputStream() = ByteArrayInputStream(data).also { read = true }
    override fun getCipherSuite() = "TLS_TEST"
    override fun getLocalCertificates(): Array<Certificate>? = null
    override fun getServerCertificates(): Array<Certificate> = emptyArray()
}
class ConfigDownloadTests {
    private val fixture = JSONObject(javaClass.getResourceAsStream("/config.json")!!.readBytes().toString(Charsets.UTF_8))
    private val envelope get() = fixture.getString("aes-256-gcm").toByteArray()
    private val password get() = fixture.getString("password")
    private val address = "https://cdn.example.com/config.enc.json"
    private fun rejects(work: () -> Unit) { try { work(); fail("invalid download accepted") } catch (_: MailException) { } }
    @Test fun downloadDecryptAndHTTPSRedirectIntegration() {
        val connections = mutableListOf<ConfigConnection>()
        val found = RemoteConfig.fetch(address,password) { uri ->
            ConfigConnection(uri.toURL(),envelope,if (connections.isEmpty()) 302 else 200,location="https://other.example.com/config.enc.json").also { connections.add(it) }
        }
        assertEquals(2,found.accounts.size); assertEquals(2,connections.size)
        assertTrue(connections.all { it.closed && !it.useCaches && !it.instanceFollowRedirects && it.getRequestProperty("Cache-Control") == "no-cache" })
        assertFalse(connections[0].read); assertTrue(connections[1].read)
    }
    @Test fun downgradeAndRedirectLoopRejectedBeforeRequest() {
        var calls = 0
        rejects { RemoteConfig.fetch(address,password) { uri -> calls++; ConfigConnection(uri.toURL(),envelope,302,location="http://other.example.com/config") } }
        assertEquals(1,calls)
        calls = 0
        rejects { RemoteConfig.fetch(address,password) { uri -> calls++; ConfigConnection(uri.toURL(),envelope,302,location=address) } }
        assertEquals(6,calls)
    }
    @Test fun httpErrorsAndDeclaredStreamedSizeLimits() {
        val denied = ConfigConnection(URL(address),envelope,403)
        rejects { RemoteConfig.fetch(address,password) { denied } }; assertFalse(denied.read); assertTrue(denied.closed)
        val large = ConfigConnection(URL(address),envelope,length=ConfigEncryption.LIMIT+1L)
        rejects { RemoteConfig.fetch(address,password) { large } }; assertFalse(large.read); assertTrue(large.closed)
        val stream = ConfigConnection(URL(address),ByteArray(ConfigEncryption.LIMIT+1))
        rejects { RemoteConfig.fetch(address,password) { stream } }; assertTrue(stream.read && stream.closed)
    }
}
