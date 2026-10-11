package com.jianxin.mail

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class ConfigTests {
    private val fixture = JSONObject(javaClass.getResourceAsStream("/config.json")!!.readBytes().toString(Charsets.UTF_8))
    private val envelope get() = fixture.getJSONObject("envelope").toString().toByteArray()
    private val password get() = fixture.getString("password")
    private val plain get() = fixture.getString("plaintext").toByteArray()
    private fun rejects(work: () -> Unit) { try { work(); fail("invalid configuration accepted") } catch (_: MailException) { } }
    @Test fun nodeUnicodeCryptoInteroperability() { assertArrayEquals(plain,ConfigEncryption.decrypt(envelope,password,ConfigCipher.LEGACY_JSON)) }
    @Test fun fileReadDecryptAndImportCompleteAccounts() {
        for (method in ConfigCipher.entries) {
            val bytes = if (method == ConfigCipher.LEGACY_JSON) envelope else fixture.getString(method.code).toByteArray()
            val found = bytes.inputStream().use { ConfigFile.read(it,password,method) }
            val expected = JianxinConfig.parse(plain).accounts
            assertEquals(expected.map { it.copy(id = "") },found.accounts.map { it.copy(id = "") })
            rejects { bytes.inputStream().use { ConfigFile.read(it,"wrong",method) } }
        }
    }
    @Test fun fileStreamSizeEmptyAndReadErrors() {
        rejects { ByteArray(ConfigEncryption.LIMIT+1).inputStream().use { ConfigFile.read(it,password,ConfigCipher.AES256) } }
        rejects { byteArrayOf().inputStream().use { ConfigFile.read(it,password,ConfigCipher.AES256) } }
        val broken = object : java.io.InputStream() { override fun read(): Int = throw java.io.IOException("unreadable document") }
        try { ConfigFile.read(broken,password,ConfigCipher.AES256); fail("read error ignored") } catch (_: java.io.IOException) { }
    }
    @Test fun wrongPasswordTamperingAndParametersRejected() {
        rejects { ConfigEncryption.decrypt(envelope,"wrong",ConfigCipher.LEGACY_JSON) }
        val modified = JSONObject(envelope.toString(Charsets.UTF_8)).put("tag","AAAAAAAAAAAAAAAAAAAAAA==")
        rejects { ConfigEncryption.decrypt(modified.toString().toByteArray(),password,ConfigCipher.LEGACY_JSON) }
        modified.put("iterations",600001)
        rejects { ConfigEncryption.decrypt(modified.toString().toByteArray(),password,ConfigCipher.LEGACY_JSON) }
        rejects { ConfigEncryption.decrypt(ByteArray(ConfigEncryption.LIMIT+1),password) }
        rejects { ConfigEncryption.decrypt(envelope,"") }
    }
    @Test fun pureCiphertextAlgorithmsAndTamperRejection() {
        for (method in listOf(ConfigCipher.AES256,ConfigCipher.AES128)) {
            val raw = fixture.getString(method.code).toByteArray()
            assertArrayEquals(plain,ConfigEncryption.decrypt(raw,password,method))
            rejects { ConfigEncryption.decrypt(raw,password,if (method == ConfigCipher.AES256) ConfigCipher.AES128 else ConfigCipher.AES256) }
            rejects { ConfigEncryption.decrypt(raw,"wrong",method) }
            val packed = java.util.Base64.getDecoder().decode(raw.toString(Charsets.UTF_8).trim())
            for (offset in listOf(0,12,packed.size-1)) {
                val changed = packed.copyOf(); changed[offset] = (changed[offset].toInt() xor 1).toByte()
                rejects { ConfigEncryption.decrypt(java.util.Base64.getEncoder().encode(changed),password,method) }
            }
        }
        rejects { ConfigEncryption.decrypt(envelope,password) }
        rejects { ConfigEncryption.decrypt("not-base64?!".toByteArray(),password) }
    }
    @Test fun schemaPresetsDistinctUsernamesAndRoles() {
        val preview = JianxinConfig.parse(plain)
        assertEquals(2,preview.accounts.size); assertEquals(1,preview.warnings.size)
        val qq = preview.accounts[0]; val custom = preview.accounts[1]
        assertEquals("imap.qq.com",qq.imapHost); assertEquals(465,qq.smtpPort); assertEquals("sample-authorization-code",qq.credential)
        assertTrue(custom.imapStartTLS && custom.smtpStartTLS); assertEquals("imap-login",custom.imapLogin); assertEquals("smtp-login",custom.smtpLogin)
        assertEquals("Sent Items",custom.folderOverrides?.get("sent"))
        assertEquals(custom,MailAccount.parse(custom.json()))
    }
    @Test fun mergePreservesIdsAndUnrelatedAccounts() {
        val qq = JianxinConfig.parse(plain).accounts[0]
        val other = MailAccount(email="other@example.com",credential="saved",imapHost="imap.example.com",smtpHost="smtp.example.com")
        val incoming = qq.copy(id="new-id",credential="new-password")
        val merged = MailAccount.merge(listOf(incoming),listOf(qq,other))
        assertEquals(2,merged.size); assertEquals(qq.id,merged[0].id); assertEquals("new-password",merged[0].credential); assertEquals(other,merged[1])
    }
    @Test fun oldStoredAccountsStillLoad() {
        val record = MailAccount(email="old@qq.com").json().apply { remove("imapUsername"); remove("smtpUsername"); remove("folderOverrides") }
        assertEquals("old@qq.com",MailAccount.parse(record).imapLogin)
    }
    @Test fun desktopCredentialReferencesAndPlaintextTLSRejected() {
        val root = JSONObject(plain.toString(Charsets.UTF_8))
        root.getJSONArray("accounts").getJSONObject(0).put("auth",JSONObject().put("type","app_password").put("password",JSONObject().put("command","never execute this")))
        val preview = JianxinConfig.parse(root.toString().toByteArray())
        assertEquals(1,preview.accounts.size); assertTrue(preview.warnings.any { it.contains("凭据引用") })
        root.getJSONArray("accounts").getJSONObject(1).getJSONObject("imap").put("tls","none")
        assertTrue(JianxinConfig.parse(root.toString().toByteArray()).accounts.isEmpty())
    }
    @Test fun duplicateAccountsRejectedAndHTTPSOnly() {
        val row = JSONObject().put("email","a@qq.com").put("auth",JSONObject().put("type","password").put("password","sample"))
        val root = JSONObject().put("accounts",org.json.JSONArray().put(row).put(row))
        assertEquals(1,JianxinConfig.parse(root.toString().toByteArray()).accounts.size)
        for (url in listOf("http://example.com/a","https://user:password@example.com/a","https://example.com/a#fragment","file:///tmp/config")) rejects { RemoteConfig.url(url) }
        assertEquals("cdn.example.com",RemoteConfig.url("https://cdn.example.com/config.enc.json?version=2").host)
    }
}
