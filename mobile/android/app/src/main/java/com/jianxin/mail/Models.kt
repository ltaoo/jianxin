package com.jianxin.mail

import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID
import java.util.Base64

fun JSONObject.text(key: String, fallback: String = ""): String = if (isNull(key)) fallback else optString(key, fallback)
fun <T> JSONArray.items(parse: (JSONObject) -> T): List<T> = (0 until length()).map { parse(getJSONObject(it)) }
fun jsonArray(items: List<JSONObject>) = JSONArray().apply { items.forEach { put(it) } }

open class MailException(message: String) : Exception(message)
class UncertainDelivery : Exception("服务器可能已收到邮件。先检查已发送邮件，再决定是否重发，避免重复发送。")

data class MailAccount(
    val id: String = UUID.randomUUID().toString(), var email: String = "", var displayName: String = "",
    var username: String = "", var credential: String = "", var oauth2: Boolean = false,
    var imapHost: String = "", var imapPort: Int = 993, var imapStartTLS: Boolean = false,
    var smtpHost: String = "", var smtpPort: Int = 465, var smtpStartTLS: Boolean = false,
    var imapUsername: String? = null, var smtpUsername: String? = null, var folderOverrides: Map<String,String>? = null,
    var oauthSettings: OAuthSettings? = null, var platform: String = "", var provider: String = ""
) {
    val title get() = displayName.ifBlank { email }
    val login get() = username.ifBlank { email }
    val imapLogin get() = imapUsername ?: login
    val smtpLogin get() = smtpUsername ?: login
    val needsAuthorization get() = oauth2 && credential.isEmpty() && oauthSettings?.refreshToken.isNullOrEmpty()
    val importIdentity get() = email.lowercase(java.util.Locale.ROOT) + "\n" + imapHost.lowercase(java.util.Locale.ROOT) + "\n" + imapLogin
    fun validate(requireCredential: Boolean = false) {
        if (!Address.valid(email) || (credential.isEmpty() && (!oauth2 || requireCredential)) || listOf(imapHost, smtpHost).any { it.isBlank() || it.any(Char::isWhitespace) } ||
            imapPort !in 1..65535 || smtpPort !in 1..65535 || listOf(login,imapLogin,smtpLogin).any { it.isEmpty() || it.any { c -> c == '\r' || c == '\n' || c == '\u0000' } } ||
            credential.contains('\u0000') || folderOverrides?.values?.any { it.isEmpty() || it.contains('\r') || it.contains('\n') } == true) throw MailException("检查邮箱地址、授权码和服务器设置")
    }
    fun preset(name: String) {
        val platforms = mapOf("QQ" to "qq","163" to "163","126" to "126","Gmail" to "gmail","阿里云" to "aliyun","Outlook" to "outlook")
        platforms[name]?.let { platform = it; provider = it }
        if (name == "Outlook") { imapHost = "outlook.office365.com"; imapPort = 993; imapStartTLS = false; smtpHost = "smtp-mail.outlook.com"; smtpPort = 587; smtpStartTLS = true; oauth2 = true; oauthSettings = oauthSettings ?: OAuthSettings(); return }
        val hosts = mapOf("QQ" to ("imap.qq.com" to "smtp.qq.com"), "163" to ("imap.163.com" to "smtp.163.com"), "126" to ("imap.126.com" to "smtp.126.com"),
            "Gmail" to ("imap.gmail.com" to "smtp.gmail.com"), "阿里云" to ("imap.aliyun.com" to "smtp.aliyun.com"))
        hosts[name]?.let { imapHost = it.first; smtpHost = it.second; imapPort = 993; smtpPort = 465; imapStartTLS = false; smtpStartTLS = false; oauth2 = false; oauthSettings = null }
    }
    fun json() = JSONObject().put("id", id).put("email", email).put("displayName", displayName).put("username", username).put("credential", credential)
        .put("oauth2", oauth2).put("imapHost", imapHost).put("imapPort", imapPort).put("imapStartTLS", imapStartTLS).put("smtpHost", smtpHost).put("smtpPort", smtpPort).put("smtpStartTLS", smtpStartTLS)
        .put("imapUsername",imapUsername).put("smtpUsername",smtpUsername).put("folderOverrides",folderOverrides?.let { JSONObject(it) }).put("oauthSettings",oauthSettings?.json()).put("platform",platform).put("provider",provider)
    companion object {
        fun parse(j: JSONObject) = MailAccount(j.text("id"), j.text("email"), j.text("displayName"), j.text("username"), j.text("credential"), j.optBoolean("oauth2"),
            j.text("imapHost"), j.optInt("imapPort", 993), j.optBoolean("imapStartTLS"), j.text("smtpHost"), j.optInt("smtpPort", 465), j.optBoolean("smtpStartTLS"),
            j.opt("imapUsername") as? String,j.opt("smtpUsername") as? String,j.optJSONObject("folderOverrides")?.let { roles -> roles.keys().asSequence().associateWith { roles.getString(it) } },
            j.optJSONObject("oauthSettings")?.let(OAuthSettings::parse),j.text("platform"),j.text("provider"))
        fun merge(incoming: List<MailAccount>, existing: List<MailAccount>): List<MailAccount> {
            val next = existing.toMutableList()
            for (account in incoming) {
                val index = next.indexOfFirst { it.importIdentity == account.importIdentity }
                if (index >= 0) {
                    val previous = next[index]
                    // Importing a pending OAuth configuration again must not erase a phone login.
                    val sameOAuth = account.oauth2 && previous.oauth2 && account.oauthSettings?.clientId == previous.oauthSettings?.clientId &&
                        account.oauthSettings?.tokenEndpoint == previous.oauthSettings?.tokenEndpoint && account.oauthSettings?.scopes == previous.oauthSettings?.scopes
                    next[index] = if (sameOAuth && account.credential.isEmpty()) account.copy(id = previous.id,credential = previous.credential,
                        oauthSettings = account.oauthSettings?.copy(refreshToken = previous.oauthSettings?.refreshToken ?: "",expiresAt = previous.oauthSettings?.expiresAt ?: 0))
                        else account.copy(id = previous.id)
                } else next.add(account)
            }
            return next
        }
    }
}
data class Folder(val path: String, val name: String, val role: String, val total: Long? = null) {
    val title get() = mapOf("inbox" to "收件箱", "sent" to "已发送", "drafts" to "草稿箱", "trash" to "已删除", "spam" to "垃圾邮件", "archive" to "归档")[role] ?: name
    fun json() = JSONObject().put("path",path).put("name",name).put("role",role).put("total",total)
    companion object { fun parse(j: JSONObject) = Folder(j.text("path"), j.text("name"), j.text("role"),if (j.isNull("total")) null else j.optLong("total",-1).takeIf { it >= 0 }) }
}
data class Address(val email: String, val name: String = "") {
    val title get() = name.ifBlank { email }
    fun json() = JSONObject().put("email", email).put("name", name)
    companion object {
        fun valid(value: String) = Regex("^[^\\s<>@,;]+@[^\\s<>@,;]+$").matches(value)
        fun parse(j: JSONObject) = Address(j.text("email"), j.text("name"))
        fun list(value: String): List<Address> {
            val chunks = mutableListOf<String>(); var part = StringBuilder(); var quoted = false; var angle = 0; var escape = false
            for (c in value) {
                if (escape) { part.append(c); escape = false; continue }
                if (c == '\\' && quoted) { escape = true; part.append(c); continue }
                if (c == '"') quoted = !quoted
                if (c == '<' && !quoted) angle++
                if (c == '>' && !quoted) angle = maxOf(0, angle - 1)
                if (c in ",;，；" && !quoted && angle == 0) { chunks.add(part.toString()); part = StringBuilder() } else part.append(c)
            }
            chunks.add(part.toString())
            return chunks.map { it.trim() }.filter { it.isNotEmpty() }.map {
                val match = Regex("^(.*)<([^>]+)>.*$").find(it)
                if (match != null) Address(match.groupValues[2].trim(), Mime.decodeHeader(match.groupValues[1].trim().trim('"'))) else Address(it)
            }
        }
    }
}
data class Attachment(val id: String = UUID.randomUUID().toString(), val filename: String, val mimeType: String, val size: Int, val data: ByteArray? = null) {
    fun json() = JSONObject().put("id",id).put("filename",filename).put("mimeType",mimeType).put("size",size).apply { data?.let { put("data",Base64.getEncoder().encodeToString(it)) } }
    companion object { fun parse(j: JSONObject) = Attachment(j.text("id"),j.text("filename"),j.text("mimeType"),j.optInt("size"), if (j.has("data")) Base64.getDecoder().decode(j.text("data")) else null) }
}
data class Message(
    val accountId: String, val folder: String, val validity: String, val uid: String,
    var subject: String = "", var from: List<Address> = emptyList(), var to: List<Address> = emptyList(), var cc: List<Address> = emptyList(), var replyTo: List<Address> = emptyList(),
    var messageId: String = "", var references: List<String> = emptyList(), var date: Long = 0L,
    var isRead: Boolean = false, var isStarred: Boolean = false, var body: String? = null, var attachments: List<Attachment> = emptyList(),
    var threadId: String = "", var inReplyTo: String = "", var threadIndexRoot: String = ""
) {
    val id get() = "$accountId:$folder:$validity:$uid"
    val sender get() = from.firstOrNull()?.title ?: "未知发件人"
    val title get() = subject.ifBlank { "（无主题）" }
    fun headerOnly() = copy(body = null,attachments = emptyList())
    fun json() = JSONObject().put("accountId",accountId).put("folder",folder).put("validity",validity).put("uid",uid).put("subject",subject)
        .put("from",jsonArray(from.map { it.json() })).put("to",jsonArray(to.map { it.json() })).put("cc",jsonArray(cc.map { it.json() })).put("replyTo",jsonArray(replyTo.map { it.json() }))
        .put("messageId",messageId).put("references",JSONArray(references)).put("date",date).put("dateVersion",1).put("isRead",isRead).put("isStarred",isStarred).put("body",body)
        .put("attachments",jsonArray(attachments.map { it.json() })).put("threadId",threadId).put("inReplyTo",inReplyTo).put("threadIndexRoot",threadIndexRoot)
    companion object {
        fun parse(j: JSONObject) = Message(j.text("accountId"),j.text("folder"),j.text("validity"),j.text("uid"),j.text("subject"),
            (j.optJSONArray("from") ?: JSONArray()).items(Address::parse),(j.optJSONArray("to") ?: JSONArray()).items(Address::parse),
            (j.optJSONArray("cc") ?: JSONArray()).items(Address::parse),(j.optJSONArray("replyTo") ?: JSONArray()).items(Address::parse),
            j.text("messageId"),(j.optJSONArray("references") ?: JSONArray()).let { a -> (0 until a.length()).map { a.getString(it) } }, if (j.optInt("dateVersion") >= 1) j.optLong("date") else 0L,
            j.optBoolean("isRead"),j.optBoolean("isStarred"),if (j.isNull("body")) null else j.text("body"),(j.optJSONArray("attachments") ?: JSONArray()).items(Attachment::parse),j.text("threadId"),j.text("inReplyTo"),j.text("threadIndexRoot"))
    }
}
data class Draft(val id: String = UUID.randomUUID().toString(), var accountId: String, var to: String = "", var cc: String = "", var bcc: String = "", var subject: String = "", var text: String = "",
    var inReplyTo: String = "", var references: List<String> = emptyList(), var attachments: List<Attachment> = emptyList(), var delivery: String = "draft", var error: String = "", val date: Long = System.currentTimeMillis()) {
    val title get() = subject.ifBlank { "未命名草稿" }
    val recipients get() = Address.list("$to,$cc,$bcc")
    val hasContent get() = to.isNotEmpty() || text.isNotEmpty() || subject.isNotEmpty() || attachments.isNotEmpty()
    fun validate() { if (recipients.isEmpty() || recipients.any { !Address.valid(it.email) } || listOf(subject,to,cc,bcc,inReplyTo).any { it.contains('\r') || it.contains('\n') } || references.any { it.contains('\r') || it.contains('\n') }) throw MailException("检查收件人地址和主题") }
    fun json() = JSONObject().put("id",id).put("accountId",accountId).put("to",to).put("cc",cc).put("bcc",bcc).put("subject",subject).put("text",text)
        .put("inReplyTo",inReplyTo).put("references",JSONArray(references)).put("attachments",jsonArray(attachments.map { it.json() })).put("delivery",delivery).put("error",error).put("date",date)
    companion object { fun parse(j: JSONObject) = Draft(j.text("id"),j.text("accountId"),j.text("to"),j.text("cc"),j.text("bcc"),j.text("subject"),j.text("text"),j.text("inReplyTo"),
        (j.optJSONArray("references") ?: JSONArray()).let { a -> (0 until a.length()).map { a.getString(it) } },(j.optJSONArray("attachments") ?: JSONArray()).items(Attachment::parse),j.text("delivery","draft"),j.text("error"),j.optLong("date")) }
}
