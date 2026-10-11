package com.jianxin.mail

import java.io.Closeable
import java.util.Base64
import java.util.Locale

data class ImapRecord(var line: String, val literals: MutableList<ByteArray> = mutableListOf())
data class ImapReply(val records: List<ImapRecord>) { val text get() = records.joinToString("\n") { it.line } }
class ImapClient(private val account: MailAccount, private val wire: MailWire = MailSocket(account.imapHost, account.imapPort, !account.imapStartTLS)) : Closeable {
    private var serial = 0
    private var capabilities = emptySet<String>()
    init {
        try {
            val greeting = wire.readLine().uppercase(Locale.ROOT)
            if (!greeting.startsWith("* OK") && !greeting.startsWith("* PREAUTH")) throw MailException("IMAP 服务器拒绝连接")
            if (account.imapStartTLS) { command("STARTTLS"); wire.startTLS() }
            if (!greeting.startsWith("* PREAUTH")) {
                if (account.oauth2) command("AUTHENTICATE XOAUTH2", Base64.getEncoder().encode("user=${account.imapLogin}\u0001auth=Bearer ${account.credential}\u0001\u0001".toByteArray()))
                else command("LOGIN ${quote(account.imapLogin)} ${quote(account.credential)}", authentication = true)
            }
            capabilities = command("CAPABILITY").text.uppercase(Locale.ROOT).split(Regex("\\s+")).toSet()
            if ("ID" in capabilities) command("ID (\"name\" \"Jianxin\" \"version\" \"1.0\")")
        } catch (e: Exception) { wire.close(); throw e }
    }
    fun command(text: String, continuation: ByteArray? = null, authentication: Boolean = false): ImapReply {
        val tag = "J%04d".format(Locale.US, ++serial); wire.line("$tag $text")
        val records = mutableListOf<ImapRecord>(); var supplied = false; var total = 0; var continuations = 0
        while (true) {
            val line = wire.readLine()
            if (line.startsWith("$tag ")) {
                if (!line.uppercase(Locale.ROOT).startsWith("$tag OK")) throw MailException(if (authentication || text.startsWith("AUTHENTICATE")) "邮箱登录失败，检查授权码或 OAuth 令牌" else "邮箱不支持或拒绝该操作，请刷新后重试")
                return ImapReply(records)
            }
            if (line.startsWith("+")) {
                if (++continuations > 3) throw MailException("邮箱认证响应异常")
                if (continuation != null && !supplied) { wire.write(continuation); wire.line(""); supplied = true } else wire.line("")
                continue
            }
            val count = Regex("\\{([0-9]+)\\+?}$").find(line)?.groupValues?.get(1)?.toIntOrNull()
            var literal: ByteArray? = null
            if (count != null) { total += count; if (total > MailSocket.MAX_MESSAGE) throw MailException("邮件响应超过 25 MB"); literal = wire.readBytes(count) }
            if (line.startsWith("* ") || records.isEmpty()) records.add(ImapRecord(line)) else records.last().line += " $line"
            literal?.let { records.last().literals.add(it) }
            if (records.size > 100000) throw MailException("邮箱响应过长")
        }
    }
    fun folders(): List<Folder> = command("LIST \"\" \"*\"").records.mapNotNull { record ->
        if (!record.line.startsWith("* LIST ", true)) return@mapNotNull null
        val close = record.line.indexOf(')'); if (close < 0) return@mapNotNull null
        val flags = record.line.substring(0, close).lowercase(Locale.ROOT); if (flags.contains("\\noselect")) return@mapNotNull null
        val tokens = tokens(record.line.substring(close + 1))
        val path = record.literals.firstOrNull()?.toString(Charsets.UTF_8) ?: tokens.getOrNull(1) ?: return@mapNotNull null
        val name = decodeMailbox(path); Folder(path, name, account.folderOverrides?.entries?.firstOrNull { it.value == path || it.value == name }?.key ?: role(name, flags))
    }.sortedWith(compareBy<Folder> { if (it.role == "inbox") 0 else 1 }.thenBy { it.name })
    fun select(folder: String, expected: String? = null): String {
        val validity = capture(command("SELECT ${quote(folder)}").text, "UIDVALIDITY ([0-9]+)") ?: throw MailException("服务器未返回 UIDVALIDITY，无法安全读取邮件")
        if (expected != null && expected != validity) throw MailException("邮箱索引已更新，请刷新列表后重新打开邮件")
        return validity
    }
    fun page(folder: String, query: String, unread: Boolean, offset: Int): Pair<List<Message>, Boolean> {
        val validity = select(folder)
        var filter = if (unread) "UNSEEN" else "ALL"
        if (query.isNotBlank()) filter += " TEXT ${quote(query)}"
        val response = command("UID SEARCH ${if (query.isEmpty()) "" else "CHARSET UTF-8 "}$filter")
        val ids = response.records.filter { it.line.startsWith("* SEARCH") }.flatMap { it.line.split(' ').drop(2) }.mapNotNull { it.toLongOrNull() }.distinct().sortedDescending()
        val page = ids.drop(offset).take(30); if (page.isEmpty()) return emptyList<Message>() to false
        val reply = command("UID FETCH ${page.joinToString(",")} (UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (SUBJECT FROM TO CC REPLY-TO DATE MESSAGE-ID REFERENCES IN-REPLY-TO)])")
        val messages = reply.records.mapNotNull { record ->
            val uid = capture(record.line, "UID ([0-9]+)") ?: return@mapNotNull null
            val raw = record.literals.firstOrNull() ?: return@mapNotNull null
            val m = Mime.summary(raw, account.id, folder, validity, uid); val flags = capture(record.line, "FLAGS \\(([^)]*)\\)") ?: ""
            m.isRead = flags.contains("\\Seen"); m.isStarred = flags.contains("\\Flagged"); m
        }
        val order = page.mapIndexed { i, uid -> uid.toString() to i }.toMap()
        return messages.sortedBy { order[it.uid] ?: 0 } to (offset + page.size < ids.size)
    }
    fun detail(summary: Message): Message {
        select(summary.folder, summary.validity)
        val record = command("UID FETCH ${summary.uid} (UID FLAGS RFC822.SIZE BODY.PEEK[])").records.firstOrNull { capture(it.line, "UID ([0-9]+)") == summary.uid }
        val raw = record?.literals?.firstOrNull() ?: throw MailException("邮件已不存在，请刷新列表")
        val m = Mime.summary(raw, account.id, summary.folder, summary.validity, summary.uid); val parsed = Mime.parse(raw)
        m.body = parsed.text; m.attachments = parsed.attachments
        val flags = capture(record.line, "FLAGS \\(([^)]*)\\)") ?: ""; m.isRead = flags.contains("\\Seen"); m.isStarred = flags.contains("\\Flagged")
        return m
    }
    fun flag(message: Message, flag: String, value: Boolean) { select(message.folder, message.validity); command("UID STORE ${message.uid} ${if (value) "+" else "-"}FLAGS.SILENT ($flag)") }
    fun move(message: Message, folder: String) {
        select(message.folder, message.validity)
        if (folder == message.folder) return
        if ("MOVE" in capabilities) command("UID MOVE ${message.uid} ${quote(folder)}")
        else if ("UIDPLUS" in capabilities) {
            command("UID COPY ${message.uid} ${quote(folder)}")
            command("UID STORE ${message.uid} +FLAGS.SILENT (\\Deleted)")
            command("UID EXPUNGE ${message.uid}")
        } else throw MailException("此服务器不支持安全移动邮件")
    }
    fun append(bytes: ByteArray, folder: String) {
        Mime.split(bytes).first["message-id"]?.let { id ->
            select(folder)
            if (command("UID SEARCH HEADER Message-ID ${quote(id)}").records.any { it.line.startsWith("* SEARCH ") && it.line.split(' ').size > 2 }) return
        }
        command("APPEND ${quote(folder)} (\\Seen) {${bytes.size}}", bytes)
    }
    override fun close() { wire.close() }
    companion object {
        fun quote(value: String) = "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"").replace("\r", "").replace("\n", "") + "\""
        fun capture(text: String, pattern: String) = Regex(pattern, RegexOption.IGNORE_CASE).find(text)?.groupValues?.get(1)
        fun tokens(text: String): List<String> {
            val values = mutableListOf<String>(); var value = StringBuilder(); var quoted = false; var escape = false; var started = false
            for (c in text) {
                if (escape) { value.append(c); escape = false; continue }
                if (c == '\\' && quoted) { escape = true; continue }
                if (c == '"') { quoted = !quoted; started = true; continue }
                if (c.isWhitespace() && !quoted) { if (started) { values.add(value.toString()); value = StringBuilder(); started = false } } else { value.append(c); started = true }
            }
            if (started) values.add(value.toString()); return values
        }
        fun decodeMailbox(path: String): String = Regex("&([^-]*)-").replace(path) { m ->
            if (m.groupValues[1].isEmpty()) "&" else try { Base64.getDecoder().decode(m.groupValues[1].replace(',', '/')).toString(Charsets.UTF_16BE) } catch (_: Exception) { m.value }
        }
        fun role(name: String, flags: String): String {
            val n = name.lowercase(Locale.ROOT); if (n == "inbox") return "inbox"
            val roles = listOf(Triple("\\sent", listOf("sent", "已发送", "发件箱"), "sent"), Triple("\\drafts", listOf("draft", "草稿"), "drafts"),
                Triple("\\trash", listOf("trash", "deleted", "已删除", "垃圾箱"), "trash"), Triple("\\junk", listOf("junk", "spam", "垃圾邮件"), "spam"), Triple("\\archive", listOf("archive", "归档"), "archive"))
            return roles.firstOrNull { flags.contains(it.first) || it.second.any(n::contains) }?.third ?: "custom"
        }
    }
}
