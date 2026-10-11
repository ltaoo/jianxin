package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test
import java.util.Base64

class MailThreadTests {
    private fun mail(uid: String, account: String = "a", folder: String = "INBOX") = Message(account,folder,"7",uid,date = uid.toLong())
    @Test fun providerThreadIdsAndLatestUnreadState() {
        val a = mail("1").copy(threadId = "gmail-thrid:42",isRead = false,subject = "First")
        val b = mail("2").copy(threadId = "gmail-thrid:42",isRead = true,subject = "Second",isStarred = true)
        val thread = MailThreads.group(listOf(b,a,a)).single()
        assertEquals(listOf(a,b),thread.messages); assertEquals(b,thread.latest); assertTrue(thread.unread); assertTrue(thread.starred)
    }
    @Test fun threadIdsAndHeadersCannotCrossAccounts() {
        val a = mail("1").copy(threadId = "gmail-thrid:42",messageId = "<same>",subject = "Same")
        assertEquals(2,MailThreads.group(listOf(a,a.copy(accountId = "b"))).size)
    }
    @Test fun referencesJoinRepliesAcrossFoldersAndMissingParents() {
        val a = mail("1").copy(messageId = "<a>",subject = "Root")
        val b = mail("2",folder = "Sent").copy(messageId = "<b>",inReplyTo = "<a>",subject = "Different")
        val c = mail("3").copy(messageId = "<c>",references = listOf("<a>","<missing>"),subject = "Third")
        assertEquals(3,MailThreads.group(listOf(c,b,a)).single().messages.size)
        assertEquals(1,MailThreads.group(listOf(b,c)).size)
    }
    @Test fun imapThreadUidIsScopedToFolderButHeadersCanLinkIt() {
        val a = mail("1").copy(threadId = "imap-thread:7:1",messageId = "<a>")
        val b = mail("1",folder = "Sent").copy(threadId = "imap-thread:7:1",messageId = "<b>")
        assertEquals(2,MailThreads.group(listOf(a,b)).size)
        assertEquals(1,MailThreads.group(listOf(a,b.copy(inReplyTo = "<a>"))).size)
    }
    @Test fun outlookIndexAndWebSubjectFallback() {
        val root = ByteArray(16) { it.toByte() }
        val index = Base64.getEncoder().encodeToString(root + byteArrayOf(42,43))
        val raw = "Thread-Index: $index\r\nSubject: First\r\n\r\n".toByteArray()
        val a = Mime.summary(raw,"a","INBOX","7","1")
        assertEquals(32,a.threadIndexRoot.length)
        assertEquals(1,MailThreads.group(listOf(a,mail("2").copy(threadIndexRoot = a.threadIndexRoot,subject = "Second"))).size)
        assertEquals("Topic",MailThreads.subject(" Re: [dev] Fwd: 答复: Topic "))
        assertEquals("[PATCH v2] Topic",MailThreads.subject("[PATCH v2] Topic"))
        assertEquals("[bug]fix",MailThreads.subject("[bug]fix"))
        assertEquals(1,MailThreads.group(listOf(mail("1").copy(subject = "Topic"),mail("2").copy(subject = "Re: Topic"))).size)
        assertEquals(2,MailThreads.group(listOf(mail("1"),mail("2"))).size)
        assertEquals("",MailThreads.threadIndexRoot("broken base64"))
    }
    @Test fun cachedMessagesKeepThreadMetadataAndOldCacheRemainsReadable() {
        val original = mail("1").copy(threadId = "gmail-thrid:3",inReplyTo = "<parent>",threadIndexRoot = "01".repeat(16))
        assertEquals(original,Message.parse(original.json()))
        val old = original.json().apply { remove("threadId"); remove("inReplyTo"); remove("threadIndexRoot") }
        assertEquals("",Message.parse(old).threadId)
    }
    @Test fun repliesUseReplyToDeduplicateAndKeepMimeThreadHeaders() {
        val parent = mail("1").copy(subject = "Topic",from = listOf(Address("sender@example.com")),replyTo = listOf(Address("reply@example.com")),
            to = listOf(Address("me@example.com"),Address("REPLY@example.com"),Address("other@example.com")),
            cc = listOf(Address("OTHER@example.com"),Address("copy@example.com")),messageId = "<parent>",references = listOf("<root>"),body = "Original text")
        val draft = MailReplies.draft(parent,"me@example.com",true)
        assertEquals("a",draft.accountId); assertEquals("reply@example.com",draft.to); assertEquals("other@example.com, copy@example.com",draft.cc)
        assertEquals(listOf("<root>","<parent>"),draft.references); assertTrue(draft.text.contains("Original text"))
        val headers = Mime.split(Mime.compose(draft,MailAccount(id = "a",email = "me@example.com",credential = "synthetic",imapHost = "imap.example.com",smtpHost = "smtp.example.com"))).first
        assertEquals("<parent>",headers["in-reply-to"]); assertEquals("<root> <parent>",headers["references"])
        assertEquals("",MailReplies.draft(parent,"me@example.com",false).cc)
        val own = parent.copy(from = listOf(Address("ME@example.com")),replyTo = emptyList(),to = listOf(Address("other@example.com")))
        assertEquals("other@example.com",MailReplies.draft(own,"me@example.com",false).to)
    }
}
