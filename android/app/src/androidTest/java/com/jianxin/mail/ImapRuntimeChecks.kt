package com.jianxin.mail

import android.app.Activity
import android.app.Instrumentation
import android.os.Bundle
import java.util.ArrayDeque

// Run against the device's actual regex and charset implementations without test libraries.
class ImapRuntimeChecks : Instrumentation() {
    override fun onCreate(arguments: Bundle?) { super.onCreate(arguments); start() }
    override fun onStart() {
        val checks = listOf("literalMarkers" to ::literalMarkers,"loginAndListFolders" to ::loginAndListFolders,"fetchHeaderLiteral" to ::fetchHeaderLiteral,"threadGroupingAndReply" to ::threadGroupingAndReply,"mailDateFallback" to ::mailDateFallback,"cancelStalledRead" to ::cancelStalledRead,"concurrentSecureStorage" to ::concurrentSecureStorage,"folderCountsWithoutExtraRequest" to ::folderCountsWithoutExtraRequest,"syncLogStorage" to ::syncLogStorage)
        var failures = 0
        checks.forEachIndexed { index,(name,run) ->
            val report = Bundle().apply {
                putString("id","InstrumentationTestRunner"); putInt("numtests",checks.size); putInt("current",index+1)
                putString("class",this@ImapRuntimeChecks.javaClass.name); putString("test",name)
            }
            sendStatus(1,report)
            try { run(); sendStatus(0,report) }
            catch (error: Throwable) { failures++; report.putString("stack",error.stackTraceToString()); sendStatus(-2,report) }
        }
        finish(Activity.RESULT_OK,Bundle().apply { putString("stream","\nAndroid IMAP runtime checks: ${checks.size - failures}/${checks.size} passed\n") })
    }
    private fun literalMarkers() {
        check(ImapClient.literalSize("* 2 FETCH (BODY[] {12}") == 12)
        check(ImapClient.literalSize("* 2 FETCH (BODY[] {12+}") == 12)
        check(ImapClient.literalSize("* LIST () NIL {0}") == 0)
        check(ImapClient.literalSize("* CAPABILITY IMAP4rev1") == null)
    }
    private fun mailDateFallback() {
        val expected = java.time.OffsetDateTime.parse("2026-09-25T09:27:50+08:00").toInstant().toEpochMilli()
        check(MailDate.resolve("","25-Sep-2026 09:27:50 +0800") == expected)
        check(MailDate.header("Fri, 25 Sep 2026 09:27:50 +0800 (CST)") == expected)
        check(MailDate.resolve("invalid","invalid") == 0L)
    }
    private fun cancelStalledRead() {
        java.net.ServerSocket(0).use { server ->
            val pool = java.util.concurrent.Executors.newSingleThreadExecutor()
            val reading = java.util.concurrent.CountDownLatch(1); val token = MailCancellation()
            try {
                val task = pool.submit<Boolean> {
                    try { MailCancellation.run(token) { MailSocket("127.0.0.1",server.localPort,false).use { reading.countDown(); it.readLine() } }; false }
                    catch (_: Exception) { true }
                }
                server.accept().use { check(reading.await(3,java.util.concurrent.TimeUnit.SECONDS)); token.cancel(); check(task.get(3,java.util.concurrent.TimeUnit.SECONDS)) }
            } finally { token.cancel(); pool.shutdownNow() }
        }
    }
    private fun concurrentSecureStorage() {
        val key = "runtime-check:${java.util.UUID.randomUUID()}"
        val store = SecureStore(targetContext); val values = (0..4).map { "中文-$it-" + "x".repeat(32768) }
        val pool = java.util.concurrent.Executors.newFixedThreadPool(4)
        try {
            store.write(key,values[0])
            val tasks = (1..4).map { index -> pool.submit {
                val independentStore = SecureStore(targetContext)
                repeat(3) { independentStore.write(key,values[index]); check(independentStore.read(key) in values) }
            } }
            tasks.forEach { it.get(10,java.util.concurrent.TimeUnit.SECONDS) }
            check(store.read(key) in values)
        } finally { pool.shutdownNow(); store.remove(key) }
    }
    private fun syncLogStorage() {
        val directory = java.io.File(targetContext.cacheDir,"sync-log-check:${java.util.UUID.randomUUID()}").apply { mkdirs() }
        val context = object : android.content.ContextWrapper(targetContext) {
            override fun getApplicationContext(): android.content.Context = this
            override fun getNoBackupFilesDir(): java.io.File = directory
        }
        try {
            val store = SecureStore(context)
            val repository = MailRepository(context)
            val first = account(); val second = account().copy(id = java.util.UUID.randomUUID().toString(),email = "another@example.com")
            val pending = account().copy(email = "pending@example.com",credential = "",oauth2 = true)
            repository.importAccounts(listOf(first,second,pending))
            check(repository.syncLogs(first.id).isEmpty())
            val reads = listOf<() -> Unit>({ repository.inboxBatch(pending.id,"",false,0) },{ repository.page(pending.id,"INBOX","",false,0) },{ repository.folders(pending.id) })
            reads.forEach { read -> try { read(); error("Expected authorization failure") } catch (_: OAuthLoginRequired) { } }
            check(repository.syncLogs(pending.id).size == 3 && repository.syncLogs(pending.id).all { it.result == MailSyncResult.AUTH_REQUIRED })
            try { repository.page(pending.id,"INBOX","",false,30); error("Expected authorization failure") } catch (_: OAuthLoginRequired) { }
            check(repository.syncLogs(pending.id).size == 3)
            val history = (1..MailSyncLog.RETENTION).reversed().map { MailSyncLog(it.toLong(),"收信",MailSyncResult.SUCCESS) }
            store.write("sync-logs:${first.id}",jsonArray(history.map { it.json() }).toString())
            val failed = MailSyncLog(1001,"收信",MailSyncResult.NETWORK_FAILED)
            repository.recordSyncLog(first.id,failed)
            repository.recordSyncLog(second.id,MailSyncLog(1002,"刷新文件夹",MailSyncResult.SUCCESS))
            val restored = MailRepository(context)
            check(restored.syncLogs(first.id).size == MailSyncLog.RETENTION)
            check(restored.syncLogs(first.id).first() == failed && restored.syncLogs(first.id).last().completedAt == 2L)
            check(restored.syncLogs(second.id).single().completedAt == 1002L)
            restored.removeAccount(first.id)
            check(store.read("sync-logs:${first.id}") == null)
            restored.recordSyncLog(first.id,failed)
            check(store.read("sync-logs:${first.id}") == null)
            check(restored.syncLogs(second.id).size == 1)
        } finally { directory.deleteRecursively() }
    }
    private fun folderCountsWithoutExtraRequest() {
        val wire = RuntimeWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 LIST-STATUS","J0002 OK cap",
            "* LIST () \"/\" \"INBOX\"","* STATUS \"INBOX\" (MESSAGES 17)","* LIST (\\Sent) \"/\" \"Sent Items\"","* STATUS \"Sent Items\" (MESSAGES 0)","J0003 OK list"))
        ImapClient(account(),wire).use { client -> check(client.folders().map { it.total } == listOf(17L,0L)) }
        check(wire.commands.size == 3 && wire.commands.last() == "J0003 LIST \"\" \"*\" RETURN (STATUS (MESSAGES))")
    }
    private fun account() = MailAccount(email="sample@example.com",credential="synthetic-password",imapHost="imap.example.com",smtpHost="smtp.example.com")
    private fun threadGroupingAndReply() {
        val parent = Message("a","INBOX","7","1",subject = "Topic",messageId = "<root>",from = listOf(Address("other@example.com")),body = "正文")
        val reply = parent.copy(uid = "2",folder = "Sent",subject = "Re: Topic",messageId = "<reply>",inReplyTo = "<root>")
        check(MailThreads.group(listOf(parent,reply)).single().messages.size == 2)
        check(MailThreads.subject("Re: [list] Topic") == "Topic")
        check(ImapClient.parseThreadHints("* THREAD (1 (2 3)) (4)","7")["3"] == "imap-thread:7:1")
        val draft = MailReplies.draft(parent,"sample@example.com",false)
        val headers = Mime.split(Mime.compose(draft,account())).first
        check(headers["in-reply-to"] == "<root>" && headers["references"] == "<root>")
        check(draft.to == "other@example.com")
    }
    private fun loginAndListFolders() {
        val wire = RuntimeWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 ID","J0002 OK cap",
            "* ID (\"name\" \"test server\")","J0003 OK identified","* LIST (\\HasNoChildren) \"/\" \"INBOX\"","J0004 OK list"))
        ImapClient(account(),wire).use { imap -> check(imap.folders().single().role == "inbox") }
        check(wire.closed)
    }
    private fun fetchHeaderLiteral() {
        val header = "Subject: =?UTF-8?B?5L2g5aW9?=\r\nFrom: Alice <alice@example.com>\r\n\r\n".toByteArray()
        val wire = RuntimeWire(listOf("* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1","J0002 OK cap",
            "* OK [UIDVALIDITY 7] valid","J0003 OK select","* SEARCH 2","J0004 OK search",
            "* 2 FETCH (BODY[HEADER.FIELDS (SUBJECT)] {${header.size}}"," UID 2 INTERNALDATE \"25-Sep-2026 09:27:50 +0800\" FLAGS (\\Seen))","J0005 OK fetch"),listOf(header))
        ImapClient(account(),wire).use { imap ->
            val message = imap.page("INBOX","",false,0).first.single()
            check(message.uid == "2" && message.subject == "你好" && message.isRead)
            check(message.date == java.time.OffsetDateTime.parse("2026-09-25T09:27:50+08:00").toInstant().toEpochMilli())
        }
        check(wire.closed)
    }
}

private class RuntimeWire(lines: List<String>, literals: List<ByteArray> = emptyList()) : MailWire {
    private val lines = ArrayDeque(lines)
    private val literals = ArrayDeque(literals)
    var closed = false
    val commands = mutableListOf<String>()
    override fun line(value: String) { commands.add(value) }
    override fun write(data: ByteArray) { }
    override fun readLine(): String = lines.removeFirst()
    override fun readBytes(count: Int): ByteArray = literals.removeFirst().also { check(it.size == count) }
    override fun startTLS() { }
    override fun close() { closed = true }
}
