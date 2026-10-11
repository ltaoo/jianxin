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
    @Test fun folderCountsUseTheExistingListRequestOnlyWhenSupported() {
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 LIST-STATUS","J0002 OK cap",
            "* LIST () \"/\" \"INBOX\"","* STATUS \"inbox\" (MESSAGES 17)","* LIST (\\Sent) \"/\" \"Sent Items\"",
            "* STATUS \"Sent Items\" (MESSAGES 0)","* LIST () \"/\" \"Other\"","J0003 OK list"))
        ImapClient(MailAccount(email="a@example.com",credential="synthetic",imapHost="imap.example.com",smtpHost="smtp.example.com"),wire).use { client ->
            val folders = client.folders()
            assertEquals(17L,folders.first { it.role == "inbox" }.total)
            assertEquals(0L,folders.first { it.role == "sent" }.total)
            assertNull(folders.first { it.path == "Other" }.total)
            assertEquals(17L,Folder.parse(folders.first { it.role == "inbox" }.json()).total)
        }
        assertEquals(3,wire.sent.size)
        assertEquals("J0003 LIST \"\" \"*\" RETURN (STATUS (MESSAGES))\r\n",wire.sent.last())
        assertFalse(wire.sent.any { it.startsWith("J0004") })
    }
    @Test fun serversWithoutListStatusHaveNoCountingRequests() {
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1","J0002 OK cap","* LIST () \"/\" \"INBOX\"","J0003 OK list"))
        ImapClient(MailAccount(email="a@example.com",credential="synthetic",imapHost="imap.example.com",smtpHost="smtp.example.com"),wire).use { assertNull(it.folders().single().total) }
        assertEquals(3,wire.sent.size); assertEquals("J0003 LIST \"\" \"*\"\r\n",wire.sent.last())
        assertNull(Folder.parse(org.json.JSONObject("""{"path":"INBOX","name":"Inbox","role":"inbox"}""")).total)
    }
    private val account = MailAccount(email = "a@example.com",credential = "app-pass",imapHost = "imap.example.com",smtpHost = "smtp.example.com")
    private fun draft() = Draft(accountId = account.id,to = "bob@example.com",cc = "cc@example.com",bcc = "secret@example.com",subject = "主题",text = "你好\n正文",inReplyTo = "<parent@example.com>",references = listOf("<parent@example.com>"))
    @Test fun imapLiteralSizeMarkers() {
        assertEquals(123,ImapClient.literalSize("* 2 FETCH (BODY[] {123}"))
        assertEquals(123,ImapClient.literalSize("* 2 FETCH (BODY[] {123+}"))
        assertEquals(0,ImapClient.literalSize("* LIST () NIL {0}"))
        for (line in listOf("* CAPABILITY IMAP4rev1","* 2 FETCH (BODY[] {123","* 2 FETCH (BODY[] 123}","* 2 FETCH (BODY[] {x}","* 2 FETCH (BODY[] {123} trailing","* 2 FETCH (BODY[] {999999999999999999999}")) assertNull(ImapClient.literalSize(line))
    }
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
            "* 2 FETCH (BODY[HEADER.FIELDS (SUBJECT)] {${raw.size}}"," UID 2 INTERNALDATE \"25-Sep-2026 09:27:50 +0800\" FLAGS (\\Seen \\Flagged))","J0005 OK fetch"),listOf(raw))
        ImapClient(account,wire).use { imap ->
            val messages = imap.page("INBOX","",false,0).first
            assertEquals(1,messages.size); assertEquals("2",messages[0].uid); assertEquals("你好",messages[0].subject); assertTrue(messages[0].isRead); assertTrue(messages[0].isStarred)
            assertEquals(java.time.OffsetDateTime.parse("2026-09-25T09:27:50+08:00").toInstant().toEpochMilli(),messages[0].date)
        }; assertTrue(wire.closed)
    }
    @Test fun uidValidityResetNeverMutatesStaleUID() {
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1","J0002 OK cap","* OK [UIDVALIDITY 8] valid","J0003 OK select"))
        ImapClient(account,wire).use { imap -> try { imap.flag(Message(account.id,"INBOX","7","2"),"\\Seen",false); fail("stale uid mutated") } catch (_: MailException) { } }
        assertFalse(wire.sent.any { it.contains("UID STORE") })
    }
    @Test fun fullMessageWithoutDateKeepsServerDateInReader() {
        val raw = "Subject: Example\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n正文".toByteArray()
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1","J0002 OK cap","* OK [UIDVALIDITY 7] valid","J0003 OK select",
            "* 2 FETCH (BODY[] {${raw.size}}"," UID 2 INTERNALDATE \"25-Sep-2026 09:27:50 +0800\" FLAGS (\\Seen))","J0004 OK fetch"),listOf(raw))
        ImapClient(account,wire).use { client ->
            val message = client.detail(Message(account.id,"INBOX","7","2"))
            assertEquals("正文",message.body)
            assertEquals(java.time.OffsetDateTime.parse("2026-09-25T09:27:50+08:00").toInstant().toEpochMilli(),message.date)
        }
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
    @Test fun rfcThreadHintsParseNestedGroupsAndDummyRoots() {
        val hints = ImapClient.parseThreadHints("* THREAD (5 (7 9)(11 12)) (0 (20 21)) (30)","42")
        assertEquals("imap-thread:42:5",hints["12"]); assertEquals("imap-thread:42:20",hints["21"])
        assertEquals("imap-thread:42:30",hints["30"]); assertNull(hints["0"])
    }
    @Test fun gmailFetchAndConversationSearchUseServerThreadId() {
        val one = "Subject: First\r\nMessage-ID: <a>\r\n\r\n".toByteArray()
        val two = "Subject: Second\r\nMessage-ID: <b>\r\n\r\n".toByteArray()
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 X-GM-EXT-1","J0002 OK cap",
            "* OK [UIDVALIDITY 7] valid","J0003 OK select","* SEARCH 2","J0004 OK search",
            "* 2 FETCH (UID 2 FLAGS (\\Seen) X-GM-THRID 123 BODY[HEADER] {${two.size}}",
            ")","J0005 OK fetch","* OK [UIDVALIDITY 7] valid","J0006 OK select","* SEARCH 1 2","J0007 OK search",
            "* 1 FETCH (UID 1 FLAGS () X-GM-THRID 123 BODY[HEADER] {${one.size}}",
            ")","* 2 FETCH (UID 2 FLAGS (\\Seen) X-GM-THRID 123 BODY[HEADER] {${two.size}}",
            ")","J0008 OK fetch"),listOf(two,one,two))
        ImapClient(account,wire).use { imap ->
            val summary = imap.page("INBOX","",false,0).first.single()
            assertEquals("gmail-thrid:123",summary.threadId)
            val related = imap.threadHeaders(summary)
            assertEquals(2,MailThreads.group(related).single().messages.size)
            assertTrue(wire.sent.contains("J0007 UID SEARCH X-GM-THRID 123\r\n"))
            assertTrue(wire.sent.any { it.contains("X-GM-THRID BODY.PEEK") })
        }
    }
    @Test fun crossFolderConversationUsesDestinationValidityAndNoSourceUid() {
        val raw = "Subject: Re: Topic\r\nMessage-ID: <reply>\r\nIn-Reply-To: <root>\r\n\r\n".toByteArray()
        val wire = ScriptWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1","J0002 OK cap",
            "* OK [UIDVALIDITY 9] valid","J0003 OK select","* SEARCH 4","J0004 OK search",
            "* 4 FETCH (UID 4 FLAGS (\\Seen) BODY[HEADER] {${raw.size}}",
            ")","J0005 OK fetch"),listOf(raw))
        ImapClient(account,wire).use { imap ->
            val root = Message(account.id,"INBOX","7","2",subject = "Topic",messageId = "<root>")
            val related = imap.threadHeaders(root,"Sent").single()
            assertEquals("9",related.validity); assertEquals("Sent",related.folder); assertEquals("<root>",related.inReplyTo)
            assertEquals(1,MailThreads.group(listOf(root,related)).size)
        }
        assertTrue(wire.sent.any { it.contains("HEADER In-Reply-To") })
        assertTrue(wire.sent.any { it.startsWith("J0005 UID FETCH 4 (") })
        assertFalse(wire.sent.any { it.contains("UID FETCH 4,2") })
    }

}
