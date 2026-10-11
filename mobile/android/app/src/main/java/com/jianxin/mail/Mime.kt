package com.jianxin.mail

import java.nio.charset.Charset
import java.util.Base64
import java.text.SimpleDateFormat
import java.util.Locale
import java.util.Date
import java.net.URLDecoder
import java.net.URLEncoder

object Mime {
    data class Content(val text: String, val attachments: List<Attachment>)
    fun split(data: ByteArray): Pair<Map<String, String>, ByteArray> {
        val raw = data.toString(Charsets.ISO_8859_1)
        var index = raw.indexOf("\r\n\r\n"); var length = 4
        if (index < 0) { index = raw.indexOf("\n\n"); length = 2 }
        val header = if (index < 0) data else data.copyOfRange(0, index)
        val body = if (index < 0) byteArrayOf() else data.copyOfRange(index + length, data.size)
        val unfolded = decode(header, "utf-8").replace("\r\n", "\n").replace(Regex("\n[ \t]+"), " ")
        val headers = mutableMapOf<String, String>()
        unfolded.lines().forEach { line ->
            val colon = line.indexOf(':')
            if (colon > 0) { val key = line.substring(0, colon).lowercase(Locale.ROOT); val value = line.substring(colon + 1).trim(); headers[key] = headers[key]?.let { "$it, $value" } ?: value }
        }
        return headers to body
    }
    fun decode(data: ByteArray, charset: String): String = try { data.toString(Charset.forName(charset)) } catch (_: Exception) { data.toString(Charsets.UTF_8) }
    fun quotedPrintable(raw: ByteArray, header: Boolean = false): ByteArray {
        val out = java.io.ByteArrayOutputStream(); var i = 0
        while (i < raw.size) {
            val c = raw[i].toInt() and 255
            if (header && c == 95) { out.write(32); i++; continue }
            if (c == 61) {
                if (i + 1 < raw.size && raw[i + 1].toInt() == 10) { i += 2; continue }
                if (i + 2 < raw.size && raw[i + 1].toInt() == 13 && raw[i + 2].toInt() == 10) { i += 3; continue }
                if (i + 2 < raw.size) {
                    val hex = raw.copyOfRange(i + 1, i + 3).toString(Charsets.US_ASCII).toIntOrNull(16)
                    if (hex != null) { out.write(hex); i += 3; continue }
                }
            }
            out.write(c); i++
        }
        return out.toByteArray()
    }
    fun decodeHeader(input: String): String {
        val joined = input.replace(Regex("(\\?=)[ \t\r\n]+(=\\?)"), "$1$2")
        return Regex("=\\?([^?]+)\\?([bBqQ])\\?([^?]*)\\?=").replace(joined) { match ->
            val data = if (match.groupValues[2].equals("b", true)) try { Base64.getMimeDecoder().decode(match.groupValues[3]) } catch (_: Exception) { byteArrayOf() }
                else quotedPrintable(match.groupValues[3].toByteArray(Charsets.US_ASCII), true)
            decode(data, match.groupValues[1])
        }
    }
    fun parameter(value: String, key: String): String? {
        val match = Regex("(?:^|;)\\s*${Regex.escape(key)}\\s*=\\s*(?:\"([^\"]*)\"|([^;\\s]+))", RegexOption.IGNORE_CASE).find(value) ?: return null
        return match.groups[1]?.value ?: match.groups[2]?.value
    }
    private fun filename(value: String): String? {
        for (key in listOf("filename","name")) {
            val segments = mutableListOf<String>()
            for (i in 0 until 50) {
                val part = parameter(value,"$key*$i*") ?: parameter(value,"$key*$i") ?: break
                segments.add(part)
            }
            val extended = if (segments.isNotEmpty()) segments.joinToString("") else parameter(value,"$key*")
            if (extended != null) {
                val parts = extended.split("'", limit = 3)
                val raw = if (parts.size == 3) parts[2] else extended
                return try { URLDecoder.decode(raw.replace("+", "%2B"), if (parts.size == 3) parts[0] else "UTF-8") } catch (_: Exception) { raw }
            }
            parameter(value,key)?.let { return decodeHeader(it) }
        }
        return null
    }
    fun summary(raw: ByteArray, account: String, folder: String, validity: String, uid: String): Message {
        val h = split(raw).first
        val m = Message(account, folder, validity, uid, subject = decodeHeader(h["subject"] ?: ""), from = Address.list(h["from"] ?: ""),
            to = Address.list(h["to"] ?: ""), cc = Address.list(h["cc"] ?: ""), replyTo = Address.list(h["reply-to"] ?: ""),
            messageId = h["message-id"] ?: "", references = (h["references"] ?: "").split(Regex("\\s+")).filter { it.isNotBlank() })
        val date = (h["date"] ?: "").replace(Regex("\\s*\\([^)]*\\)"), "")
        for (format in listOf("EEE, d MMM yyyy HH:mm:ss Z", "d MMM yyyy HH:mm:ss Z", "EEE, d MMM yyyy HH:mm Z", "d MMM yyyy HH:mm Z")) {
            try { SimpleDateFormat(format, Locale.US).parse(date)?.let { m.date = it.time; return m } } catch (_: Exception) { }
        }
        return m
    }
    fun parse(raw: ByteArray, depth: Int = 0): Content {
        if (depth >= 16) throw MailException("邮件 MIME 嵌套过深")
        val (h, body) = split(raw)
        val contentType = h["content-type"] ?: "text/plain"; val type = contentType.lowercase(Locale.ROOT); val disposition = h["content-disposition"] ?: ""
        val boundary = parameter(contentType, "boundary")
        if (type.startsWith("multipart/") && boundary != null) {
            val rawBody = body.toString(Charsets.ISO_8859_1)
            val delimiter = Regex("(?m)^--${Regex.escape(boundary)}(--)?[ \t]*(?:\r?\n|$)")
            val markers = delimiter.findAll(rawBody).toList()
            val children = (0 until maxOf(0, markers.size - 1)).take(200).mapNotNull { i ->
                if (markers[i].groupValues[1] == "--") null else {
                    val part = rawBody.substring(markers[i].range.last + 1, markers[i + 1].range.first).trimEnd('\r', '\n').toByteArray(Charsets.ISO_8859_1)
                    parse(part, depth + 1)
                }
            }
            val attachments = children.flatMap { it.attachments }
            if (attachments.size > 100) throw MailException("邮件附件过多")
            val text = if (type.startsWith("multipart/alternative")) children.firstOrNull { it.text.isNotEmpty() }?.text ?: "" else children.map { it.text }.filter { it.isNotEmpty() }.joinToString("\n\n")
            return Content(text, attachments)
        }
        val bytes = when ((h["content-transfer-encoding"] ?: "").lowercase(Locale.ROOT)) {
            "base64" -> try { Base64.getMimeDecoder().decode(body) } catch (_: Exception) { throw MailException("邮件附件编码无效") }
            "quoted-printable" -> quotedPrintable(body)
            else -> body
        }
        val name = filename(disposition) ?: filename(contentType)
        if (disposition.lowercase(Locale.ROOT).startsWith("attachment") || name != null || (!type.startsWith("text/plain") && !type.startsWith("text/html"))) {
            return Content("", listOf(Attachment(filename = name ?: "attachment", mimeType = type.substringBefore(';'), size = bytes.size, data = bytes)))
        }
        val text = decode(bytes, parameter(contentType, "charset") ?: "utf-8")
        return Content(if (type.startsWith("text/html")) htmlText(text) else text, emptyList())
    }
    fun htmlText(html: String): String {
        var text = html.replace(Regex("<(script|style|head)\\b[^>]*>[\\s\\S]*?</\\1\\s*>", RegexOption.IGNORE_CASE), "")
            .replace(Regex("</?(p|div|br|li|tr|h[1-6]|blockquote|pre)\\b[^>]*>", RegexOption.IGNORE_CASE), "\n").replace(Regex("<[^>]*>"), "")
        val entities = mapOf("amp" to "&", "lt" to "<", "gt" to ">", "quot" to "\"", "apos" to "'", "nbsp" to " ", "ndash" to "–", "mdash" to "—", "hellip" to "…", "copy" to "©")
        text = Regex("&(#x[0-9a-fA-F]+|#[0-9]+|[A-Za-z]+);").replace(text) { m ->
            val key = m.groupValues[1]
            val point = if (key.startsWith("#x")) key.drop(2).toIntOrNull(16) else if (key.startsWith("#")) key.drop(1).toIntOrNull() else null
            entities[key] ?: if (point != null && Character.isValidCodePoint(point) && point !in 0xd800..0xdfff) String(Character.toChars(point)) else m.value
        }
        return text.replace(Regex("\n{3,}"), "\n\n").trim()
    }
    fun encodedHeader(value: String): String {
        // Fold encoded words at Unicode character boundaries below RFC 2047's 75-byte limit.
        val chunks = mutableListOf<String>(); var part = StringBuilder(); var size = 0
        value.codePoints().forEach { point ->
            val c = String(Character.toChars(point)); val bytes = c.toByteArray().size
            if (size + bytes > 42) { chunks.add(part.toString()); part = StringBuilder(); size = 0 }
            part.append(c); size += bytes
        }
        if (part.isNotEmpty() || chunks.isEmpty()) chunks.add(part.toString())
        return chunks.joinToString("\r\n ") { "=?UTF-8?B?${Base64.getEncoder().encodeToString(it.toByteArray())}?=" }
    }
    fun wrap(data: ByteArray): String = Base64.getEncoder().encodeToString(data).chunked(76).joinToString("\r\n")
    fun compose(draft: Draft, account: MailAccount): ByteArray {
        draft.validate(); account.validate()
        fun addresses(list: List<Address>) = list.joinToString(", ") { (if (it.name.isEmpty()) "" else encodedHeader(it.name) + " ") + "<${it.email}>" }
        val domain = account.email.substringAfter('@', "jianxin.local")
        val headers = mutableListOf("From: ${if (account.displayName.isEmpty()) "" else encodedHeader(account.displayName) + " "}<${account.email}>",
            "To: ${addresses(Address.list(draft.to))}", "Subject: ${encodedHeader(draft.subject)}", "Date: ${SimpleDateFormat("EEE, dd MMM yyyy HH:mm:ss Z", Locale.US).format(Date(draft.date))}",
            "Message-ID: <${draft.id}@$domain>", "MIME-Version: 1.0")
        if (draft.cc.isNotEmpty()) headers.add("Cc: ${addresses(Address.list(draft.cc))}")
        if (draft.inReplyTo.isNotEmpty()) headers.add("In-Reply-To: ${draft.inReplyTo}")
        if (draft.references.isNotEmpty()) headers.add("References: ${draft.references.joinToString(" ")}")
        val text = "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n${wrap(draft.text.toByteArray())}\r\n"
        if (draft.attachments.isEmpty()) return (headers.joinToString("\r\n") + "\r\n" + text).toByteArray()
        val boundary = "jx-${draft.id}"; headers.add("Content-Type: multipart/mixed; boundary=\"$boundary\"")
        val body = StringBuilder("\r\n--$boundary\r\n$text")
        draft.attachments.forEach { a ->
            val name = URLEncoder.encode(a.filename, "UTF-8").replace("+", "%20")
            val type = if (a.mimeType.contains('\r') || a.mimeType.contains('\n')) "application/octet-stream" else a.mimeType
            body.append("--$boundary\r\nContent-Type: $type\r\nContent-Disposition: attachment; filename*=UTF-8''$name\r\nContent-Transfer-Encoding: base64\r\n\r\n${wrap(a.data ?: byteArrayOf())}\r\n")
        }
        body.append("--$boundary--\r\n")
        return (headers.joinToString("\r\n") + "\r\n" + body).toByteArray()
    }
}
