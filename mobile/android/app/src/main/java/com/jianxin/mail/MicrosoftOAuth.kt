package com.jianxin.mail

import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.net.URI
import java.net.URLEncoder
import javax.net.ssl.HttpsURLConnection

data class OAuthSettings(
    val clientId: String = "",
    val tokenEndpoint: String = MicrosoftOAuth.DEFAULT_ENDPOINT,
    val scopes: List<String> = MicrosoftOAuth.DEFAULT_SCOPES,
    val refreshToken: String = "",
    val expiresAt: Long = 0
) {
    fun json() = JSONObject().put("clientId",clientId).put("tokenEndpoint",tokenEndpoint)
        .put("scopes",org.json.JSONArray(scopes)).put("refreshToken",refreshToken).put("expiresAt",expiresAt)
    companion object {
        fun parse(value: JSONObject): OAuthSettings {
            val scopes = value.optJSONArray("scopes")
            return OAuthSettings(value.text("clientId"),value.text("tokenEndpoint",MicrosoftOAuth.DEFAULT_ENDPOINT),
                scopes?.let { (0 until it.length()).map { index -> it.getString(index) } } ?: MicrosoftOAuth.DEFAULT_SCOPES,
                value.text("refreshToken"),value.optLong("expiresAt"))
        }
    }
}

class OAuthLoginRequired : MailException("请点击邮箱完成 OAuth 授权登录")
data class OAuthTokens(val accessToken: String, val refreshToken: String, val expiresAt: Long)
class DeviceAuthorization(val settings: OAuthSettings, val deviceCode: String, val userCode: String,
    val verificationURI: String, val deadline: Long, var intervalSeconds: Long)

object MicrosoftOAuth {
    const val DEFAULT_ENDPOINT = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"
    val DEFAULT_SCOPES = listOf("offline_access","https://outlook.office.com/IMAP.AccessAsUser.All","https://outlook.office.com/SMTP.Send")
    fun endpoint(value: String): URI {
        val uri = try { URI(value) } catch (_: Exception) { throw MailException("Microsoft OAuth 令牌端点无效") }
        if (uri.scheme != "https" || uri.host != "login.microsoftonline.com" || uri.port !in listOf(-1,443) ||
            uri.rawUserInfo != null || uri.rawQuery != null || uri.rawFragment != null ||
            !Regex("^/[A-Za-z0-9.-]+/oauth2/v2\\.0/token$").matches(uri.rawPath ?: ""))
            throw MailException("手机授权需使用 Microsoft 的 HTTPS v2.0 令牌端点")
        return uri
    }
    private fun validate(settings: OAuthSettings) {
        endpoint(settings.tokenEndpoint)
        if (settings.clientId.isBlank()) throw MailException("请编辑邮箱并填写 OAuth Client ID（应用 ID）")
        if (settings.scopes.isEmpty() || settings.scopes.any { it.isBlank() || it.any(Char::isWhitespace) }) throw MailException("OAuth 权限配置无效")
    }
    fun start(settings: OAuthSettings, post: (String,Map<String,String>) -> JSONObject = ::postForm): DeviceAuthorization {
        validate(settings)
        val scopes = (settings.scopes + "offline_access").distinct()
        val result = post(settings.tokenEndpoint.removeSuffix("/token") + "/devicecode",mapOf("client_id" to settings.clientId,"scope" to scopes.joinToString(" ")))
        if (result.has("error")) throw failure(result.text("error"))
        try {
            val verification = URI(result.getString("verification_uri"))
            require(verification.scheme == "https" && verification.host in listOf("microsoft.com","www.microsoft.com","login.microsoftonline.com") &&
                verification.rawUserInfo == null && verification.rawFragment == null && verification.port in listOf(-1,443))
            val expires = result.getLong("expires_in"); require(expires in 1..3600)
            val code = result.getString("device_code"); val user = result.getString("user_code"); require(code.isNotBlank() && user.isNotBlank())
            return DeviceAuthorization(settings,code,user,verification.toString(),System.nanoTime() + expires * 1_000_000_000L,result.optLong("interval",5).coerceIn(5,60))
        } catch (_: Exception) { throw MailException("Microsoft 返回的授权信息无效，请重试") }
    }
    fun poll(session: DeviceAuthorization, post: (String,Map<String,String>) -> JSONObject = ::postForm): OAuthTokens? {
        if (System.nanoTime() >= session.deadline) throw MailException("授权码已过期，请重新开始授权")
        val result = post(session.settings.tokenEndpoint,mapOf("grant_type" to "urn:ietf:params:oauth:grant-type:device_code",
            "client_id" to session.settings.clientId,"device_code" to session.deviceCode))
        return when (val error = result.text("error")) {
            "authorization_pending" -> null
            "slow_down" -> { session.intervalSeconds += 5; null }
            "" -> tokens(result)
            else -> throw failure(error)
        }
    }
    fun refresh(settings: OAuthSettings, post: (String,Map<String,String>) -> JSONObject = ::postForm): OAuthTokens {
        validate(settings)
        if (settings.refreshToken.isEmpty()) throw OAuthLoginRequired()
        val result = post(settings.tokenEndpoint,mapOf("grant_type" to "refresh_token","client_id" to settings.clientId,
            "refresh_token" to settings.refreshToken,"scope" to (settings.scopes + "offline_access").distinct().joinToString(" ")))
        if (result.has("error")) throw failure(result.text("error"))
        return tokens(result,settings.refreshToken)
    }
    private fun tokens(result: JSONObject, previousRefresh: String = ""): OAuthTokens {
        try {
            val access = result.opt("access_token") as? String ?: throw IllegalArgumentException()
            val expires = result.getLong("expires_in")
            val refresh = if (result.has("refresh_token")) result.opt("refresh_token") as? String ?: throw IllegalArgumentException() else previousRefresh
            require(access.isNotEmpty() && refresh.isNotEmpty() && expires in 1..31_536_000 && result.text("token_type").equals("Bearer",true))
            return OAuthTokens(access,refresh,System.currentTimeMillis() + expires * 1000)
        } catch (_: Exception) { throw MailException("Microsoft 返回的令牌信息无效，请重新授权") }
    }
    private fun failure(error: String): MailException = when (error) {
        "invalid_grant", "interaction_required" -> OAuthLoginRequired()
        "authorization_declined", "access_denied" -> MailException("授权被拒绝，可以重新开始授权")
        "expired_token" -> MailException("授权码已过期，请重新开始授权")
        "invalid_client", "unauthorized_client" -> MailException("OAuth 应用配置无效，请检查 Client ID 和公共客户端授权设置")
        "invalid_scope" -> MailException("OAuth 权限配置无效，请检查 IMAP/SMTP 权限")
        else -> MailException("Microsoft 授权失败，请检查应用配置后重试")
    }
    private fun postForm(url: String, form: Map<String,String>): JSONObject {
        endpoint(if (url.endsWith("/devicecode")) url.removeSuffix("/devicecode") + "/token" else url)
        return request(URI(url),form)
    }
    private fun request(uri: URI, form: Map<String,String>): JSONObject {
        val connection = uri.toURL().openConnection() as HttpsURLConnection
        connection.connectTimeout = 30000; connection.readTimeout = 30000; connection.instanceFollowRedirects = false
        connection.useCaches = false; connection.requestMethod = "POST"; connection.doOutput = true
        connection.setRequestProperty("Content-Type","application/x-www-form-urlencoded")
        try {
            val data = form.entries.joinToString("&") { URLEncoder.encode(it.key,"UTF-8") + "=" + URLEncoder.encode(it.value,"UTF-8") }.toByteArray(Charsets.UTF_8)
            connection.setFixedLengthStreamingMode(data.size); connection.outputStream.use { it.write(data) }
            val code = connection.responseCode
            if (code !in 200..299 && code !in listOf(400,401)) throw MailException("Microsoft 授权服务暂不可用，请重试")
            val input = if (code in 200..299) connection.inputStream else connection.errorStream
            val bytes = input?.use { stream ->
                val out = ByteArrayOutputStream(); val buffer = ByteArray(8192)
                while (true) { val count = stream.read(buffer); if (count < 0) break; if (out.size() + count > 256 * 1024) throw MailException("授权响应过大"); out.write(buffer,0,count) }
                out.toByteArray()
            } ?: throw MailException("Microsoft 授权响应为空")
            return try { JSONObject(ConfigEncryption.utf8(bytes)) } finally { bytes.fill(0) }
        } catch (e: MailException) { throw e }
        catch (_: Exception) { throw MailException("无法连接 Microsoft 授权服务，请检查网络后重试") }
        finally { connection.disconnect() }
    }
}
