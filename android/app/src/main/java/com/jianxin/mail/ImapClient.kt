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
            if ("ID" in capabilities) command("ID (\"name\" \"Jianxin\" \"version\" \"${BuildConfig.VERSION_NAME}\")")
        } catch (e: Exception) { wire.close(); throw e }
    }
    fun command(text: String, continuation: ByteArray? = null, authentication: Boolean = false): ImapReply {
        MailCancellation.checkCurrent()
        val tag = "J%04d".format(Locale.US, ++serial); wire.line("$tag $text")
        val records = mutableListOf<ImapRecord>(); var supplied = false; var total = 0; var continuations = 0
        while (true) {
            MailCancellation.checkCurrent()
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
            val count = literalSize(line)
            var literal: ByteArray? = null
            if (count != null) { total += count; if (total > MailSocket.MAX_MESSAGE) throw MailException("邮件响应超过 25 MB"); literal = wire.readBytes(count) }
            if (line.startsWith("* ") || records.isEmpty()) records.add(ImapRecord(line)) else records.last().line += " $line"
            literal?.let { records.last().literals.add(it) }
            if (records.size > 100000) throw MailException("邮箱响应过长")
        }
    }
    fun folders(): List<Folder> {
        val status = if ("LIST-STATUS" in capabilities || "IMAP4REV2" in capabilities) " RETURN (STATUS (MESSAGES))" else ""
        val records = command("LIST \"\" \"*\"$status").records
        fun key(path: String) = if (path.equals("INBOX",true)) "INBOX" else path
        val counts = records.filter { it.line.startsWith("* STATUS ",true) }.mapNotNull { record ->
            val path = record.literals.firstOrNull()?.toString(Charsets.UTF_8) ?: tokens(record.line.substring(9)).firstOrNull() ?: return@mapNotNull null
            val total = capture(record.line,"MESSAGES ([0-9]+)")?.toLongOrNull() ?: return@mapNotNull null
            key(path) to total
        }.toMap()
        return records.mapNotNull { record ->
        if (!record.line.startsWith("* LIST ", true)) return@mapNotNull null
        val close = record.line.indexOf(')'); if (close < 0) return@mapNotNull null
        val flags = record.line.substring(0, close).lowercase(Locale.ROOT); if (flags.contains("\\noselect")) return@mapNotNull null
        val tokens = tokens(record.line.substring(close + 1))
        val path = record.literals.firstOrNull()?.toString(Charsets.UTF_8) ?: tokens.getOrNull(1) ?: return@mapNotNull null
        val name = decodeMailbox(path); Folder(path, name, account.folderOverrides?.entries?.firstOrNull { it.value == path || it.value == name }?.key ?: role(name, flags),counts[key(path)])
    }.sortedWith(compareBy<Folder> { if (it.role == "inbox") 0 else 1 }.thenBy { it.name })
    }
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
        val hints = threadHints(validity)
        val messages = headers(page.map { it.toString() },folder,validity,hints)
        val order = page.mapIndexed { i, uid -> uid.toString() to i }.toMap()
        return messages.sortedBy { order[it.uid] ?: 0 } to (offset + page.size < ids.size)
    }
    private fun headers(uids: List<String>, folder: String, validity: String, hints: Map<String,String> = emptyMap()): List<Message> {
        if (uids.isEmpty()) return emptyList()
        val gmail = if ("X-GM-EXT-1" in capabilities) " X-GM-THRID" else ""
        val reply = command("UID FETCH ${uids.joinToString(",")} (UID FLAGS INTERNALDATE RFC822.SIZE$gmail BODY.PEEK[HEADER.FIELDS (SUBJECT FROM TO CC REPLY-TO DATE MESSAGE-ID REFERENCES IN-REPLY-TO THREAD-INDEX)])")
        val messages = reply.records.mapNotNull { record ->
            val uid = capture(record.line, "UID ([0-9]+)") ?: return@mapNotNull null
            val raw = record.literals.firstOrNull() ?: return@mapNotNull null
            val m = Mime.summary(raw, account.id, folder, validity, uid, internalDate(record)); val flags = capture(record.line, "FLAGS \\(([^)]*)\\)") ?: ""
            m.isRead = flags.contains("\\Seen"); m.isStarred = flags.contains("\\Flagged"); m
                .also { it.threadId = capture(record.line,"X-GM-THRID ([0-9]+)")?.let { value -> "gmail-thrid:$value" } ?: hints[uid] ?: "" }
        }
        return messages
    }
    private fun threadHints(validity: String): Map<String,String> {
        if ("THREAD=REFERENCES" !in capabilities || "X-GM-EXT-1" in capabilities) return emptyMap()
        return runCatching { parseThreadHints(command("UID THREAD REFERENCES UTF-8 ALL").text,validity) }.getOrDefault(emptyMap())
    }
    fun threadHeaders(summary: Message, folder: String = summary.folder): List<Message> {
        val sameFolder = folder == summary.folder
        val validity = select(folder,if (sameFolder) summary.validity else null)
        val hints = threadHints(validity)
        val uids = if (sameFolder && summary.threadId.startsWith("imap-thread:") && hints.values.any { it == summary.threadId }) hints.filterValues { it == summary.threadId }.keys.toList()
        else {
            val criteria = if (summary.threadId.startsWith("gmail-thrid:") && "X-GM-EXT-1" in capabilities) {
                val id = summary.threadId.removePrefix("gmail-thrid:"); require(id.isNotEmpty() && id.all(Char::isDigit)); "X-GM-THRID $id"
            } else {
                val ids = (summary.references + listOf(summary.inReplyTo,summary.messageId)).flatMap { Regex("<[^<>]+>").findAll(it).map { match -> match.value }.toList() }.distinct().take(32)
                val filters = ids.flatMap { id -> listOf("HEADER Message-ID ${quote(id)}","HEADER In-Reply-To ${quote(id)}","HEADER References ${quote(id)}") }.toMutableList()
                if (summary.threadIndexRoot.length == 32) {
                    val bytes = summary.threadIndexRoot.chunked(2).map { it.toInt(16).toByte() }.take(15).toByteArray()
                    filters.add("HEADER Thread-Index ${quote(Base64.getEncoder().encodeToString(bytes))}")
                }
                MailThreads.subject(summary.subject).takeIf { it.isNotEmpty() }?.let { filters.add("HEADER Subject ${quote(it)}") }
                if (filters.isEmpty()) return if (sameFolder) listOf(summary) else emptyList()
                filters.reduce { left,right -> "OR ($left) ($right)" }
            }
            command("UID SEARCH ${if (criteria.any { it.code > 127 }) "CHARSET UTF-8 " else ""}$criteria").records.filter { it.line.startsWith("* SEARCH ") }.flatMap { it.line.split(' ').drop(2) }.filter { (it.toLongOrNull() ?: 0) > 0 }.distinct()
        }
        return (uids + if (sameFolder) listOf(summary.uid) else emptyList()).distinct().chunked(30).flatMap { headers(it,folder,validity,hints) }.sortedBy { it.date }
    }
    fun detail(summary: Message): Message {
        select(summary.folder, summary.validity)
        val record = command("UID FETCH ${summary.uid} (UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])").records.firstOrNull { capture(it.line, "UID ([0-9]+)") == summary.uid }
        val raw = record?.literals?.firstOrNull() ?: throw MailException("邮件已不存在，请刷新列表")
        val m = Mime.summary(raw, account.id, summary.folder, summary.validity, summary.uid, internalDate(record)); val parsed = Mime.parse(raw)
        if (m.date == 0L) m.date = summary.date
        m.body = parsed.text; m.attachments = parsed.attachments
        m.threadId = summary.threadId
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
        private fun internalDate(record: ImapRecord) = capture(record.line,"INTERNALDATE \"([^\"]+)\"") ?: ""
        // Android's ICU regex parser requires both literal braces to be escaped.
        private val literalPattern = Regex("\\{([0-9]+)\\+?\\}$")
        fun literalSize(line: String): Int? = literalPattern.find(line)?.groupValues?.get(1)?.toIntOrNull()
        fun parseThreadHints(text: String, validity: String): Map<String,String> {
            val result = mutableMapOf<String,String>()
            text.lineSequence().filter { it.startsWith("* THREAD",true) }.forEach { line ->
                var depth = 0; val group = mutableListOf<String>(); var index = 8
                while (index < line.length) {
                    val char = line[index]
                    if (char == '(') { if (depth == 0) group.clear(); depth++ }
                    else if (char == ')') { depth--; if (depth == 0 && group.isNotEmpty()) {
                        val min = group.mapNotNull { it.toLongOrNull() }.filter { it > 0 }.minOrNull()
                        if (min != null) group.filter { it != "0" }.forEach { result[it] = "imap-thread:$validity:$min" }
                    } }
                    else if (char.isDigit() && depth > 0) { val begin = index; while (index < line.length && line[index].isDigit()) index++; group.add(line.substring(begin,index)); continue }
                    index++
                }
            }
            return result
        }
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
