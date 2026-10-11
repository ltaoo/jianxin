package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test
import java.util.ArrayDeque
import java.io.IOException

private class ScriptWire(lines: List<String>, literals: List<ByteArray> = emptyList()) : MailWire {
    private val lines = ArrayDeque(lines)
    private val literals = ArrayDeque(literals)
    val sent = mutableListOf<String>()
    var closed = false
    var upgraded = false
    override fun line(value: String) { sent.add("$value\r\n") }
    override fun write(data: ByteArray) { sent.add(data.toString(Charsets.UTF_8)) }
    override fun readLine(): String = if (lines.isEmpty()) throw IOException("mock EOF") else lines.removeFirst()
    override fun readBytes(count: Int): ByteArray = literals.removeFirst().also { assertEquals(count,it.size) }
    override fun startTLS() { upgraded = true }
    override fun close() { closed = true }
}
class ProtocolTests {
    private val account = MailAccount(email = "a@example.com",credential = "app-pass",imapHost = "imap.example.com",smtpHost = "smtp.example.com")
    private fun draft() = Draft(accountId = account.id,to = "bob@example.com",cc = "cc@example.com",bcc = "secret@example.com",subject = "主题",text = "你好\n正文",inReplyTo = "<parent@example.com>",references = listOf("<parent@example.com>"))
    @Test fun unicodeHeadersQuotedNamesAndMailbox() {
        assertEquals("你好",Mime.decodeHeader("=?UTF-8?B?5L2g5aW9?="))
        val long = "长主题".repeat(40); assertEquals(long,Mime.decodeHeader(Mime.encodedHeader(long)))
        assertTrue(Mime.encodedHeader(long).split("\r\n ").all { it.length <= 75 })
        val list = Address.list("\"Doe, John\" <john@example.com>, Alice <a@example.com>")
        assertEquals(2,list.size); assertEquals("Doe, John",list[0].name)
        assertEquals("已发送",ImapClient.decodeMailbox("&XfJT0ZAB-"))
    }
    @Test fun nativeHtmlDropsTrackingAndScripts() {
        assertEquals("你好 & 你",Mime.htmlText("<p>你好 &amp; &#x4f60;</p><script>bad()</script><img src='https://tracker.test'><style>bad</style>"))
    }
    @Test fun multipartBinaryAndThreadingRoundTrip() {
        val binary = byteArrayOf(0,1,10,13,-1)
        val draft = draft().copy(attachments = listOf(Attachment(filename = "资料.bin",mimeType = "application/octet-stream",size = binary.size,data = binary)))
        val bytes = Mime.compose(draft,account); val content = Mime.parse(bytes); val headers = Mime.split(bytes).first
        assertFalse(headers.containsKey("bcc")); assertEquals("<parent@example.com>",headers["in-reply-to"])
        assertEquals(draft.text,content.text); assertEquals("资料.bin",content.attachments[0].filename); assertArrayEquals(binary,content.attachments[0].data)
    }
    @Test fun headerInjectionAndMIMEDepthRejected() {
        try { Mime.compose(draft().copy(subject = "bad\r\nBcc: evil@example.com"),account); fail("injection accepted") } catch (_: MailException) { }
        try { Mime.parse(byteArrayOf(),16); fail("depth limit ignored") } catch (_: MailException) { }
    }
    @Test fun imapLiteralAndPostLiteralUIDFlags() {
        val raw = "Subject: =?UTF-8?B?5L2g5aW9?=\r\nFrom: Alice <alice@example.com>\r\n\r\n".toByteArray()
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 MOVE","J0002 OK cap","* OK [UIDVALIDITY 7] valid","J0003 OK select","* SEARCH 1 2","J0004 OK search",
            "* 2 FETCH (BODY[HEADER.FIELDS (SUBJECT)] {${raw.size}}"," UID 2 FLAGS (\\Seen \\Flagged))","J0005 OK fetch"),listOf(raw))
        ImapClient(account,wire).use { imap ->
            val messages = imap.page("INBOX","",false,0).first
            assertEquals(1,messages.size); assertEquals("2",messages[0].uid); assertEquals("你好",messages[0].subject); assertTrue(messages[0].isRead); assertTrue(messages[0].isStarred)
        }; assertTrue(wire.closed)
    }
    @Test fun uidValidityResetNeverMutatesStaleUID() {
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1","J0002 OK cap","* OK [UIDVALIDITY 8] valid","J0003 OK select"))
        ImapClient(account,wire).use { imap -> try { imap.flag(Message(account.id,"INBOX","7","2"),"\\Seen",false); fail("stale uid mutated") } catch (_: MailException) { } }
        assertFalse(wire.sent.any { it.contains("UID STORE") })
    }
    @Test fun smtpEnvelopeBccAndAcceptance() {
        val wire = ScriptWire(listOf("220 hello","250-host","250 AUTH PLAIN","235 ok","250 from","250 to","250 cc","250 bcc","354 go","250 accepted"))
        SmtpClient.send(draft(),account) { wire }
        assertTrue(wire.sent.contains("RCPT TO:<secret@example.com>\r\n")); assertTrue(wire.closed)
        assertEquals("..one\r\n...two\r\n",SmtpClient.dotStuff(".one\r\n..two\r\n".toByteArray()).toString(Charsets.UTF_8))
    }
    @Test fun smtpLostAcceptanceIsUncertainAndNoAutomaticResend() {
        val wire = ScriptWire(listOf("220 hello","250 AUTH PLAIN","235 ok","250 from","250 to","250 cc","250 bcc","354 go"))
        try { SmtpClient.send(draft(),account) { wire }; fail("lost receipt accepted") } catch (_: UncertainDelivery) { }
        assertEquals(1,wire.sent.count { it == "DATA\r\n" })
    }
    @Test fun authFailureIsNotUncertain() {
        val wire = ScriptWire(listOf("220 hello","250 AUTH PLAIN","535 bad auth"))
        try { SmtpClient.send(draft(),account) { wire }; fail("auth accepted") } catch (_: MailException) { }
        assertFalse(wire.sent.contains("DATA\r\n"))
    }
    @Test fun startTlsIsRequiredBeforeSmtpAuth() {
        val wire = ScriptWire(listOf("220 hello","250-STARTTLS","250 AUTH PLAIN","220 upgrade","250 AUTH PLAIN","235 ok","250 from","250 to","250 cc","250 bcc","354 go","250 accepted"))
        SmtpClient.send(draft(),account.copy(smtpStartTLS = true)) { wire }
        assertTrue(wire.upgraded); assertTrue(wire.sent.indexOf("STARTTLS\r\n") < wire.sent.indexOfFirst { it.startsWith("AUTH") })
    }
    @Test fun rfc2231SegmentsAndWhitespaceParameters() {
        val raw = "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*0* = UTF-8''%E8%B5%84; filename*1*=%E6%96%99.bin\r\n\r\nabc".toByteArray()
        assertEquals("资料.bin",Mime.parse(raw).attachments[0].filename)
    }
    @Test fun uidPlusFallbackExpungesOnlySelectedMessage() {
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 UIDPLUS","J0002 OK cap","* OK [UIDVALIDITY 7] valid","J0003 OK select","J0004 OK copy","J0005 OK store","J0006 OK expunge"))
        ImapClient(account,wire).use { it.move(Message(account.id,"INBOX","7","2"),"Trash") }
        assertTrue(wire.sent.contains("J0006 UID EXPUNGE 2\r\n")); assertFalse(wire.sent.contains("EXPUNGE\r\n"))
    }
    @Test fun explicitDataRejectionIsFailedRatherThanUncertain() {
        val wire = ScriptWire(listOf("220 hello","250 AUTH PLAIN","235 ok","250 from","250 to","250 cc","250 bcc","354 go","550 refused"))
        try { SmtpClient.send(draft(),account) { wire }; fail("refusal accepted") } catch (_: MailException) { }
    }

}
