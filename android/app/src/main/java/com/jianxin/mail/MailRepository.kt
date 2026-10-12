package com.jianxin.mail

import android.content.Context
import org.json.JSONArray

class MailRepository(context: Context) {
    private val disk = SecureStore(context.applicationContext)
    private val folderCache = java.util.concurrent.ConcurrentHashMap<String,List<Folder>>()
    private val connectionLocks = java.util.concurrent.ConcurrentHashMap<String,java.util.concurrent.locks.ReentrantLock>()
    private val cacheLocks = java.util.concurrent.ConcurrentHashMap<String,Any>()
    private fun cacheLock(id: String) = cacheLocks.computeIfAbsent(id) { Any() }
    @Volatile var accounts = (disk.read("accounts")?.let(::JSONArray) ?: JSONArray()).items(MailAccount::parse); private set
    @Volatile var drafts = (disk.read("drafts")?.let(::JSONArray) ?: JSONArray()).items(Draft::parse); private set
    init {
        if (drafts.any { it.delivery == "sending" }) {
            drafts = drafts.map { if (it.delivery == "sending") it.copy(delivery = "uncertain", error = UncertainDelivery().message ?: "") else it }
            disk.write("drafts", jsonArray(drafts.map { it.json() }).toString())
        }
    }
    @Synchronized fun saveAccount(value: MailAccount) {
        value.validate()
        // OAuth accounts are saved before opening the browser; passwords still get verified here.
        val folders = if (value.oauth2) null else ImapClient(value).use { it.folders() }
        MailCancellation.checkCurrent()
        val next = accounts.filter { it.id != value.id } + value
        disk.write("accounts", jsonArray(next.map { it.json() }).toString()); accounts = next
        if (folders != null) cacheFolders(value.id,folders)
    }
    fun configURL() = disk.read("config-source-url") ?: ""
    fun rememberConfigURL(value: String) { disk.write("config-source-url",value) }
    fun configCipher() = ConfigCipher.entries.firstOrNull { it.code == disk.read("config-source-cipher") } ?: ConfigCipher.AES256
    fun rememberConfigCipher(value: ConfigCipher) { disk.write("config-source-cipher",value.code) }
    fun timeFormat(): String = (disk.read("time-format") ?: MailTime.DEFAULT).let { style -> if (runCatching { MailTime.validate(style) }.isSuccess) style else MailTime.DEFAULT }
    fun rememberTimeFormat(style: String) { MailTime.validate(style); disk.write("time-format",style) }
    @Synchronized fun importAccounts(values: List<MailAccount>) {
        if (values.isEmpty()) throw MailException("没有可导入的账号")
        values.forEach { it.validate() }
        val next = MailAccount.merge(values,accounts)
        disk.write("accounts",jsonArray(next.map { it.json() }).toString()); accounts = next
    }
    @Synchronized fun removeAccount(id: String) {
        if (drafts.any { it.accountId == id && it.delivery == "sending" }) throw MailException("正在发送邮件，完成后再移除邮箱")
        val next = accounts.filter { it.id != id }; disk.write("accounts",jsonArray(next.map { it.json() }).toString()); accounts = next
        val retained = drafts.filter { it.accountId != id }; disk.write("drafts",jsonArray(retained.map { it.json() }).toString()); drafts = retained
        synchronized(cacheLock(id)) {
            val keys = disk.read("keys:$id")?.let(::JSONArray) ?: JSONArray()
            (0 until keys.length()).forEach { disk.remove(keys.getString(it)) }
            disk.remove("keys:$id"); disk.remove("folders:$id"); disk.remove("sync-logs:$id")
        }
        folderCache.remove(id)
    }
    fun account(id: String) = accounts.firstOrNull { it.id == id } ?: throw MailException("邮箱已移除")
    @Synchronized fun saveOAuthTokens(id: String, settings: OAuthSettings, tokens: OAuthTokens) {
        val current = account(id)
        if (!current.oauth2 || current.oauthSettings?.clientId != settings.clientId || current.oauthSettings?.tokenEndpoint != settings.tokenEndpoint || current.oauthSettings?.scopes != settings.scopes)
            throw MailException("邮箱授权配置已更改，请重新授权")
        persistAccount(current.copy(credential = tokens.accessToken,oauthSettings = settings.copy(refreshToken = tokens.refreshToken,expiresAt = tokens.expiresAt)))
    }
    private fun persistAccount(value: MailAccount) {
        val next = accounts.map { if (it.id == value.id) value else it }
        disk.write("accounts",jsonArray(next.map { it.json() }).toString()); accounts = next
    }
    private fun connectionAccount(id: String): MailAccount {
        val lock = connectionLocks.computeIfAbsent(id) { java.util.concurrent.locks.ReentrantLock() }
        lock.lockInterruptibly()
        try {
        MailCancellation.checkCurrent()
        var value = account(id)
        if (value.needsAuthorization) throw OAuthLoginRequired()
        val settings = value.oauthSettings
        if (value.oauth2 && settings != null && settings.refreshToken.isNotEmpty() && (value.credential.isEmpty() || settings.expiresAt <= System.currentTimeMillis() + 60000)) {
            try { saveOAuthTokens(id,settings,MicrosoftOAuth.refresh(settings)); value = account(id) }
            catch (e: OAuthLoginRequired) { persistAccount(value.copy(credential = "",oauthSettings = settings.copy(refreshToken = "",expiresAt = 0))); throw e }
        }
        value.validate(requireCredential = true)
        MailCancellation.checkCurrent()
        return value
        } finally { lock.unlock() }
    }
    fun cachedFolders(id: String): List<Folder> = folderCache[id] ?: (disk.read("folders:$id")?.let(::JSONArray) ?: JSONArray()).items(Folder::parse).also { folderCache.putIfAbsent(id,it) }
    private fun cacheFolders(id: String, folders: List<Folder>) {
        MailCancellation.checkCurrent()
        disk.write("folders:$id",jsonArray(folders.map { it.json() }).toString()); folderCache[id] = folders
    }
    fun inboxPath(id: String): String {
        val value = account(id)
        val override = value.folderOverrides?.get("inbox")
        val known = cachedFolders(id)
        return known.firstOrNull { it.path == override || it.name == override }?.path ?: known.firstOrNull { it.role == "inbox" }?.path ?: override ?: "INBOX"
    }
    fun cachedInbox(id: String, query: String, unread: Boolean) = cachedList(listKey(id,inboxPath(id),query,unread))
    fun syncLogs(id: String): List<MailSyncLog> = synchronized(cacheLock(id)) {
        account(id)
        (disk.read("sync-logs:$id")?.let(::JSONArray) ?: JSONArray()).items(MailSyncLog::parse)
    }
    internal fun recordSyncLog(id: String, entry: MailSyncLog) = synchronized(cacheLock(id)) {
        if (accounts.none { it.id == id }) return@synchronized
        val history = (disk.read("sync-logs:$id")?.let(::JSONArray) ?: JSONArray()).items(MailSyncLog::parse)
        disk.write("sync-logs:$id",jsonArray((listOf(entry) + history.take(MailSyncLog.RETENTION - 1)).map { it.json() }).toString())
    }
    private fun <T> receive(id: String, operation: String, work: () -> T): T = MailSyncLog.receive(operation,{ entry ->
        // Log storage must not change a completed IMAP operation's outcome.
        try { recordSyncLog(id,entry) } catch (error: Exception) { android.util.Log.e("Jianxin","收信日志保存失败：${error.javaClass.simpleName}") }
    },work)
    fun inboxBatch(id: String, query: String, unread: Boolean, offset: Int, cancelled: () -> Boolean = { false }): InboxBatch =
        if (offset == 0) receive(id,"收信") { loadInboxBatch(id,query,unread,offset,cancelled) } else loadInboxBatch(id,query,unread,offset,cancelled)
    private fun loadInboxBatch(id: String, query: String, unread: Boolean, offset: Int, cancelled: () -> Boolean): InboxBatch {
        if (account(id).needsAuthorization) throw OAuthLoginRequired()
        return ImapClient(connectionAccount(id)).use { client ->
            if (offset == 0) {
                val discovered = client.folders()
                if (cancelled()) throw java.io.InterruptedIOException("Sync cancelled")
                cacheFolders(id,discovered)
            }
            val path = inboxPath(id)
            val result = client.page(path,query,unread,offset)
            if (cancelled()) throw java.io.InterruptedIOException("Sync cancelled")
            val key = listKey(id,path,query,unread)
            val cached = if (offset == 0) result.first else (cachedList(key) + result.first).distinctBy { it.id }
            cacheList(id,key,cached)
            InboxBatch(result.first,result.second)
        }
    }
    fun folders(id: String): List<Folder> = receive(id,"刷新文件夹") {
        val result = ImapClient(connectionAccount(id)).use { it.folders() }; cacheFolders(id,result); result
    }
    fun listKey(id: String, folder: String, query: String, unread: Boolean) = "list:$id:$folder:$query:$unread"
    fun cachedList(key: String) = (disk.read(key)?.let(::JSONArray) ?: JSONArray()).items(Message::parse)
    private fun cache(id: String, key: String, value: String) = synchronized(cacheLock(id)) {
        MailCancellation.checkCurrent()
        if (accounts.none { it.id == id }) return@synchronized
        disk.write(key,value)
        val old = disk.read("keys:$id")?.let(::JSONArray) ?: JSONArray()
        val keys = (0 until old.length()).map { old.getString(it) }.toMutableSet()
        if (keys.add(key)) disk.write("keys:$id",JSONArray(keys.toList()).toString())
    }
    fun page(id: String, folder: String, query: String, unread: Boolean, offset: Int): Pair<List<Message>,Boolean> =
        if (offset == 0) receive(id,"收信") { ImapClient(connectionAccount(id)).use { it.page(folder,query,unread,offset) } }
        else ImapClient(connectionAccount(id)).use { it.page(folder,query,unread,offset) }
    fun cacheList(id: String, key: String, list: List<Message>) { cache(id,key,jsonArray(list.map { it.headerOnly().json() }).toString()) }
    fun updateCachedMessage(key: String, flags: Message, removed: Boolean, unread: Boolean) = synchronized(cacheLock(flags.accountId)) {
        val cached = cachedList(key).map { if (it.id == flags.id) it.copy(isRead = flags.isRead,isStarred = flags.isStarred,date = flags.date) else it }
            .filter { (!removed || it.id != flags.id) && (!unread || !it.isRead) }
        cacheList(flags.accountId,key,cached)
    }
    fun cachedDetail(summary: Message): Message? = disk.read("message:${summary.id}")?.let { Message.parse(org.json.JSONObject(it)).apply { if (summary.date != 0L) date = summary.date } }
    private fun conversationCache(summary: Message): org.json.JSONObject? = disk.read("thread:${summary.id}")?.let { raw ->
        if (raw.trimStart().startsWith("[")) org.json.JSONObject().put("messages",JSONArray(raw)) else org.json.JSONObject(raw)
    }
    fun cachedConversation(summary: Message, seed: List<Message>): List<Message> =
        (seed + (conversationCache(summary)?.optJSONArray("messages") ?: JSONArray()).items(Message::parse)).distinctBy { it.id }.sortedBy { it.date }
    private fun invalidateConversations(id: String) {
        val keys = disk.read("keys:$id")?.let(::JSONArray) ?: return
        (0 until keys.length()).map { keys.getString(it) }.filter { it.startsWith("thread:") }.forEach(disk::remove)
    }
    fun conversation(summary: Message, seed: List<Message>, forceRefresh: Boolean = false, cancelled: () -> Boolean): Pair<List<Message>,Boolean> {
        MailCancellation.checkCurrent()
        val stored = conversationCache(summary)
        val cached = (seed + (stored?.optJSONArray("messages") ?: JSONArray()).items(Message::parse)).distinctBy { it.id }.sortedBy { it.date }
        val age = System.currentTimeMillis() - (stored?.optLong("updatedAt") ?: 0L)
        if (!forceRefresh && stored != null && age in 0L..120_000L) return cached to stored.optBoolean("partial")
        val found = mutableListOf<Message>()
        var partial = false
        ImapClient(connectionAccount(summary.accountId)).use { client ->
            val paths = (listOf(summary.folder) + cachedFolders(summary.accountId).map { it.path }).distinct()
            for (path in paths) {
                if (cancelled()) throw java.io.InterruptedIOException("Reader cancelled")
                try { found.addAll(client.threadHeaders(summary,path)) } catch (_: MailException) { partial = true }
            }
        }
        if (cancelled()) throw java.io.InterruptedIOException("Reader cancelled")
        val related = MailThreads.group((found + cached).distinctBy { it.id }).firstOrNull { thread -> thread.messages.any { it.id == summary.id } }?.messages ?: seed
        // A message can be present in Inbox, All Mail and Sent at the same time.
        val distinct = related.sortedBy { if (it.folder == summary.folder) 0 else 1 }.distinctBy { it.messageId.ifEmpty { it.id } }.sortedBy { it.date }
        MailCancellation.checkCurrent()
        cache(summary.accountId,"thread:${summary.id}",org.json.JSONObject().put("messages",jsonArray(distinct.map { it.headerOnly().json() }))
            .put("updatedAt",System.currentTimeMillis()).put("partial",partial).toString())
        return distinct to partial
    }
    fun detail(summary: Message, forceRefresh: Boolean = false, cached: Message? = null): Pair<Message, Boolean> {
        val key = "message:${summary.id}"
        val local = cached ?: runCatching { cachedDetail(summary) }.getOrNull()
        if (!forceRefresh && local?.body != null) {
            val mail = local.copy(isRead = summary.isRead,isStarred = summary.isStarred)
            if (!mail.isRead) {
                try {
                    ImapClient(connectionAccount(summary.accountId)).use { it.flag(mail,"\\Seen",true) }
                    MailCancellation.checkCurrent(); mail.isRead = true; cache(summary.accountId,key,mail.json().toString())
                } catch (_: Exception) { MailCancellation.checkCurrent() }
            }
            MailCancellation.checkCurrent(); return mail to true
        }
        return try {
            val result = ImapClient(connectionAccount(summary.accountId)).use { client ->
                client.detail(summary).also { mail -> if (!mail.isRead) runCatching { client.flag(mail,"\\Seen",true); mail.isRead = true } }
            }; MailCancellation.checkCurrent(); cache(summary.accountId,key,result.json().toString()); result to false
        } catch (e: Exception) {
            MailCancellation.checkCurrent()
            val fallback = local ?: throw e
            fallback to true
        }
    }
    fun flag(message: Message, read: Boolean? = null, star: Boolean? = null) {
        ImapClient(connectionAccount(message.accountId)).use { imap -> read?.let { imap.flag(message,"\\Seen",it) }; star?.let { imap.flag(message,"\\Flagged",it) } }
        read?.let { message.isRead = it }; star?.let { message.isStarred = it }
        val detail = if (message.body == null) cachedDetail(message)?.copy(isRead = message.isRead,isStarred = message.isStarred) ?: message else message
        cache(message.accountId,"message:${message.id}",detail.json().toString())
    }
    fun move(message: Message, folder: String) { ImapClient(connectionAccount(message.accountId)).use { it.move(message,folder) }; disk.remove("message:${message.id}") }
    @Synchronized fun saveDraft(draft: Draft) {
        val next = drafts.filter { it.id != draft.id } + if (draft.hasContent) listOf(draft.copy()) else emptyList()
        disk.write("drafts",jsonArray(next.map { it.json() }).toString()); drafts = next
    }
    @Synchronized fun removeDraft(id: String) { val next = drafts.filter { it.id != id }; disk.write("drafts",jsonArray(next.map { it.json() }).toString()); drafts = next }
    fun send(draft: Draft, force: Boolean = false): String {
        val selected = connectionAccount(draft.accountId); draft.validate()
        val current = drafts.firstOrNull { it.id == draft.id }
        if (current?.delivery == "sending" || current?.delivery == "sent") throw MailException("此邮件正在发送或已发送")
        if (current?.delivery == "uncertain" && !force) throw UncertainDelivery()
        saveDraft(draft.copy(delivery = "sending",error = ""))
        val bytes = try { SmtpClient.send(draft,selected) } catch (e: Exception) {
            saveDraft(draft.copy(delivery = if (e is UncertainDelivery) "uncertain" else "failed",error = friendly(e))); throw e
        }
        // SMTP acceptance is final; failures saving a Sent copy must never enable a retry.
        try { saveDraft(draft.copy(delivery = "sent",error = "")) } catch (_: Exception) {
            drafts = drafts.filter { it.id != draft.id } + draft.copy(delivery = "sent",error = "")
            return "邮件已发送；本地记录保存失败，重启后请先检查已发送邮件"
        }
        val sent = cachedFolders(selected.id).firstOrNull { it.role == "sent" }
        runCatching { invalidateConversations(selected.id) }
        if (sent != null) try { ImapClient(selected).use { it.append(bytes,sent.path) } } catch (_: Exception) { return "邮件已发送；已发送文件夹副本保存失败" }
        return "邮件已发送"
    }
    fun reply(message: Message, all: Boolean): Draft {
        return MailReplies.draft(message,account(message.accountId).email,all)
    }
    companion object {
        fun friendly(e: Exception): String = MailErrors.friendly(e)
    }
}
