package com.jianxin.mail

import org.json.JSONObject
import java.net.URI
import java.util.Base64
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import javax.net.ssl.HttpsURLConnection
import javax.crypto.Cipher
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.PBEKeySpec
import javax.crypto.spec.SecretKeySpec

data class ConfigPreview(val accounts: List<MailAccount>, val warnings: List<String>)

enum class ConfigCipher(val title: String, val code: String, val keySize: Int) {
    AES256("AES-256-GCM", "aes-256-gcm", 256), AES128("AES-128-GCM", "aes-128-gcm", 128), LEGACY_JSON("旧版 JSON（兼容）", "legacy-json", 256)
}

object ConfigEncryption {
    const val LIMIT = 2 * 1024 * 1024
    val aad = "jianxin-config:v1:pbkdf2-sha256:600000:aes-256-gcm".toByteArray(Charsets.UTF_8)
    fun decrypt(data: ByteArray, password: String, method: ConfigCipher = ConfigCipher.AES256): ByteArray {
        if (data.size > LIMIT) throw MailException("加密配置不能超过 2 MB")
        if (password.isEmpty() || password.toByteArray(Charsets.UTF_8).size > 1024) throw MailException("请输入解密密码（最多 1024 字节）")
        if (method == ConfigCipher.LEGACY_JSON) return decryptLegacy(data, password)
        val packed = try {
            val encoded = utf8(data).filterNot { it in " \t\r\n" }
            Base64.getDecoder().decode(encoded).also { require(it.size > 28 && Base64.getEncoder().encodeToString(it) == encoded) }
        } catch (_: Exception) { throw MailException("密文必须是完整的 Base64 内容；旧文件请选择旧版 JSON") }
        val key = derive(password, ("jianxin-config:password-key:v2:" + method.code).toByteArray(Charsets.UTF_8), method.keySize)
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, packed.copyOfRange(0,12)))
            cipher.updateAAD(("jianxin-config:v2:pbkdf2-sha256:600000:" + method.code).toByteArray(Charsets.UTF_8))
            return cipher.doFinal(packed.copyOfRange(12,packed.size))
        } catch (_: Exception) { throw MailException("解密失败：密码、加密方式不匹配或文件已被修改") }
        finally { key.fill(0) }
    }
    private fun derive(password: String, context: ByteArray, size: Int): ByteArray {
        val chars = password.toCharArray()
        val spec = PBEKeySpec(chars, context, 600000, size)
        return try { SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256").generateSecret(spec).encoded }
            finally { spec.clearPassword(); chars.fill('\u0000') }
    }
    private fun decryptLegacy(data: ByteArray, password: String): ByteArray {
        if (data.size > LIMIT) throw MailException("加密配置不能超过 2 MB")
        if (password.isEmpty() || password.toByteArray(Charsets.UTF_8).size > 1024) throw MailException("请输入解密密码（最多 1024 字节）")
        val envelope = try { JSONObject(utf8(data)) } catch (_: Exception) { throw MailException("加密格式不支持或文件损坏，请使用配套加密工具") }
        val fields = try {
            require(envelope.getString("format") == "jianxin-config" && envelope.get("version") == 1 && envelope.getString("kdf") == "pbkdf2-sha256" &&
                envelope.get("iterations") == 600000 && envelope.getString("cipher") == "aes-256-gcm")
            listOf("salt", "nonce", "ciphertext", "tag").map { Base64.getDecoder().decode(envelope.getString(it)) }.also {
                require(it[0].size == 16 && it[1].size == 12 && it[2].isNotEmpty() && it[3].size == 16)
            }
        } catch (_: Exception) { throw MailException("加密格式不支持或文件损坏，请使用配套加密工具") }
        val key = derive(password,fields[0],256)
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, fields[1]))
            cipher.updateAAD(aad)
            return cipher.doFinal(fields[2] + fields[3])
        } catch (_: Exception) { throw MailException("解密失败：密码错误或文件已被修改") }
        finally { key.fill(0) }
    }
    fun utf8(data: ByteArray): String = Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(data)).toString()
}

object JianxinConfig {
    fun parse(data: ByteArray): ConfigPreview {
        val root = try { JSONObject(ConfigEncryption.utf8(data)) } catch (_: Exception) { throw MailException("解密后的内容不是有效 config.json") }
        val rows = root.optJSONArray("accounts") ?: throw MailException("配置需要包含 accounts 数组")
        if (rows.length() !in 1..100) throw MailException("支持 1 至 100 个账号")
        val accounts = mutableListOf<MailAccount>(); val warnings = mutableListOf<String>(); val seen = mutableSetOf<String>()
        for (index in 0 until rows.length()) {
            try {
                val row = rows.getJSONObject(index)
                val account = MailAccount(email = row.getString("email").trim())
                if (!Address.valid(account.email)) throw MailException("邮箱地址无效")
                val auth = row.getJSONObject("auth"); val type = auth.getString("type")
                if (type !in listOf("password", "app_password", "oauth2", "o_auth2")) throw MailException("不支持此认证方式")
                account.oauth2 = type == "oauth2" || type == "o_auth2"
                val secret = auth.opt(if (account.oauth2) "access_token" else "password")
                if (secret !is String || secret.isEmpty()) throw MailException("需要字符串形式的密码或访问令牌；手机不能读取桌面端凭据引用")
                account.credential = secret; account.username = auth.text("username")
                account.displayName = row.text("display_name", row.text("id"))
                val domain = account.email.substringAfterLast('@').lowercase(java.util.Locale.ROOT)
                val preset = presets[domain] ?: listOf("imap.$domain", "smtp.$domain", "465", "tls")
                account.imapHost = preset[0]; account.smtpHost = preset[1]; account.smtpPort = preset[2].toInt(); account.smtpStartTLS = preset[3] == "starttls"
                for (kind in listOf("imap", "smtp")) {
                    if (!row.isNull(kind)) {
                        val server = row.getJSONObject(kind); val host = server.getString("host"); val port = server.get("port")
                        if (port !is Number || port.toDouble() != port.toInt().toDouble()) throw MailException("服务器需要 host 和整数 port")
                        val tls = server.text("tls", "tls")
                        if (tls !in listOf("tls", "starttls", "start_tls")) throw MailException("手机仅支持 TLS 或 STARTTLS，不能导入明文连接")
                        if (kind == "imap") { account.imapHost = host; account.imapPort = port.toInt(); account.imapStartTLS = tls != "tls"; account.imapUsername = server.opt("username") as? String }
                        else { account.smtpHost = host; account.smtpPort = port.toInt(); account.smtpStartTLS = tls != "tls"; account.smtpUsername = server.opt("username") as? String }
                    }
                }
                row.optJSONObject("folders")?.let { roles -> account.folderOverrides = listOf("inbox", "sent", "drafts", "trash", "spam", "archive").mapNotNull { key -> (roles.opt(key) as? String)?.let { key to it } }.toMap() }
                account.validate()
                if (!seen.add(account.importIdentity)) throw MailException("配置中有重复邮箱和收信服务器")
                accounts.add(account)
                if (account.oauth2) warnings.add("第 ${index + 1} 个账号使用已有 OAuth 令牌，到期后需更新配置；不导入桌面端刷新流程。")
            } catch (e: Exception) { warnings.add("第 ${index + 1} 个账号未导入：${if (e is MailException) e.message else "账号字段格式无效"}") }
        }
        if (root.has("general")) warnings.add("仅导入邮箱账号。桌面端 general 设置（含 API、Agent、同步策略）不应用到手机。")
        return ConfigPreview(accounts, warnings)
    }
    private val presets: Map<String, List<String>> = buildMap {
        fun add(domains: List<String>, imap: String, smtp: String, port: String = "465", tls: String = "tls") { domains.forEach { put(it, listOf(imap, smtp, port, tls)) } }
        add(listOf("gmail.com", "googlemail.com"), "imap.gmail.com", "smtp.gmail.com")
        add(listOf("qq.com", "foxmail.com"), "imap.qq.com", "smtp.qq.com")
        listOf("163.com", "126.com", "yeah.net", "aliyun.com", "sina.com", "139.com", "zoho.com", "fastmail.com").forEach { add(listOf(it), "imap.$it", "smtp.$it") }
        add(listOf("yahoo.com"), "imap.mail.yahoo.com", "smtp.mail.yahoo.com")
        add(listOf("icloud.com", "me.com"), "imap.mail.me.com", "smtp.mail.me.com", "587", "starttls")
        add(listOf("outlook.com", "hotmail.com", "live.com", "office365.com"), "outlook.office365.com", "smtp-mail.outlook.com", "587", "starttls")
    }
}

object RemoteConfig {
    fun url(value: String): URI {
        val uri = try { URI(value) } catch (_: Exception) { throw MailException("请输入完整 HTTPS URL") }
        if (!uri.scheme.equals("https", true) || uri.host.isNullOrBlank() || uri.rawUserInfo != null || uri.rawFragment != null) throw MailException("请输入完整 HTTPS URL，不要包含用户名、密码或片段")
        return uri
    }
    fun fetch(value: String, password: String, method: ConfigCipher = ConfigCipher.AES256, open: (URI) -> HttpsURLConnection = { it.toURL().openConnection() as HttpsURLConnection }): ConfigPreview {
        var target = url(value)
        repeat(6) { hop ->
            val connection = open(target)
            connection.instanceFollowRedirects = false; connection.connectTimeout = 30000; connection.readTimeout = 30000
            connection.useCaches = false; connection.setRequestProperty("Cache-Control", "no-cache")
            try {
                val code = connection.responseCode
                if (code in listOf(301, 302, 303, 307, 308)) {
                    if (hop == 5) throw MailException("配置 URL 重定向次数过多")
                    val location = connection.getHeaderField("Location") ?: throw MailException("配置 URL 重定向无效")
                    target = url(target.resolve(location).toString())
                } else {
                    if (code !in 200..299) throw MailException("下载失败，请检查 URL 和 CDN 文件权限")
                    if (connection.contentLengthLong > ConfigEncryption.LIMIT) throw MailException("加密配置不能超过 2 MB")
                    val data = ByteArrayOutputStream()
                    val deadline = System.nanoTime() + 60_000_000_000L
                    connection.inputStream.use { input ->
                        val buffer = ByteArray(8192)
                        while (true) {
                            if (Thread.currentThread().isInterrupted) throw MailException("配置下载已取消")
                            if (System.nanoTime() > deadline) throw MailException("配置下载超时，请重试")
                            val count = input.read(buffer); if (count < 0) break
                            if (data.size() + count > ConfigEncryption.LIMIT) throw MailException("加密配置不能超过 2 MB")
                            data.write(buffer, 0, count)
                        }
                    }
                    val plaintext = ConfigEncryption.decrypt(data.toByteArray(), password, method)
                    return try { JianxinConfig.parse(plaintext) } finally { plaintext.fill(0) }
                }
            } catch (e: MailException) { throw e }
            catch (_: Exception) { throw MailException("无法下载配置，请检查网络、URL 和 HTTPS 证书后重试") }
            finally { connection.disconnect() }
        }
        throw MailException("配置 URL 重定向次数过多")
    }
}
