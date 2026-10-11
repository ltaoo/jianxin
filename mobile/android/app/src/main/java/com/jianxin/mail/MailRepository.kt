package com.jianxin.mail

import android.content.Context
import org.json.JSONArray
import java.util.Locale
import java.util.Date

class MailRepository(context: Context) {
    private val disk = SecureStore(context.applicationContext)
    var accounts = (disk.read("accounts")?.let(::JSONArray) ?: JSONArray()).items(MailAccount::parse); private set
    var drafts = (disk.read("drafts")?.let(::JSONArray) ?: JSONArray()).items(Draft::parse); private set
    init {
        if (drafts.any { it.delivery == "sending" }) {
            drafts = drafts.map { if (it.delivery == "sending") it.copy(delivery = "uncertain", error = UncertainDelivery().message ?: "") else it }
            disk.write("drafts", jsonArray(drafts.map { it.json() }).toString())
        }
    }
    @Synchronized fun saveAccount(value: MailAccount) {
        value.validate()
        val folders = ImapClient(value).use { it.folders() }
        val next = accounts.filter { it.id != value.id } + value
        disk.write("accounts", jsonArray(next.map { it.json() }).toString()); accounts = next
        disk.write("folders:${value.id}", jsonArray(folders.map { it.json() }).toString())
    }
    fun configURL() = disk.read("config-source-url") ?: ""
    fun rememberConfigURL(value: String) { disk.write("config-source-url",value) }
    fun configCipher() = ConfigCipher.entries.firstOrNull { it.code == disk.read("config-source-cipher") } ?: ConfigCipher.AES256
    fun rememberConfigCipher(value: ConfigCipher) { disk.write("config-source-cipher",value.code) }
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
        val keys = disk.read("keys:$id")?.let(::JSONArray) ?: JSONArray()
        (0 until keys.length()).forEach { disk.remove(keys.getString(it)) }
        disk.remove("keys:$id"); disk.remove("folders:$id")
    }
    fun account(id: String) = accounts.firstOrNull { it.id == id } ?: throw MailException("邮箱已移除")
    fun cachedFolders(id: String) = (disk.read("folders:$id")?.let(::JSONArray) ?: JSONArray()).items(Folder::parse)
    fun folders(id: String): List<Folder> {
        val result = ImapClient(account(id)).use { it.folders() }; disk.write("folders:$id",jsonArray(result.map { it.json() }).toString()); return result
    }
    fun listKey(id: String, folder: String, query: String, unread: Boolean) = "list:$id:$folder:$query:$unread"
    fun cachedList(key: String) = (disk.read(key)?.let(::JSONArray) ?: JSONArray()).items(Message::parse)
    @Synchronized private fun cache(id: String, key: String, value: String) {
        disk.write(key,value)
        val old = disk.read("keys:$id")?.let(::JSONArray) ?: JSONArray()
        val keys = (0 until old.length()).map { old.getString(it) }.toMutableSet()
        if (keys.add(key)) disk.write("keys:$id",JSONArray(keys.toList()).toString())
    }
    fun page(id: String, folder: String, query: String, unread: Boolean, offset: Int) = ImapClient(account(id)).use { it.page(folder, query, unread, offset) }
    fun cacheList(id: String, key: String, list: List<Message>) { cache(id,key,jsonArray(list.map { it.json() }).toString()) }
    fun cachedDetail(summary: Message): Message? = disk.read("message:${summary.id}")?.let { Message.parse(org.json.JSONObject(it)) }
    fun detail(summary: Message): Pair<Message, Boolean> {
        val key = "message:${summary.id}"
        return try {
            val result = ImapClient(account(summary.accountId)).use { it.detail(summary) }; cache(summary.accountId,key,result.json().toString()); result to false
        } catch (e: Exception) {
            val cached = disk.read(key)?.let { Message.parse(org.json.JSONObject(it)) } ?: throw e
            cached to true
        }
    }
    fun flag(message: Message, read: Boolean? = null, star: Boolean? = null) {
        ImapClient(account(message.accountId)).use { imap -> read?.let { imap.flag(message,"\\Seen",it) }; star?.let { imap.flag(message,"\\Flagged",it) } }
        read?.let { message.isRead = it }; star?.let { message.isStarred = it }
        cache(message.accountId,"message:${message.id}",message.json().toString())
    }
    fun move(message: Message, folder: String) { ImapClient(account(message.accountId)).use { it.move(message,folder) }; disk.remove("message:${message.id}") }
    @Synchronized fun saveDraft(draft: Draft) {
        val next = drafts.filter { it.id != draft.id } + if (draft.hasContent) listOf(draft.copy()) else emptyList()
        disk.write("drafts",jsonArray(next.map { it.json() }).toString()); drafts = next
    }
    @Synchronized fun removeDraft(id: String) { val next = drafts.filter { it.id != id }; disk.write("drafts",jsonArray(next.map { it.json() }).toString()); drafts = next }
    fun send(draft: Draft, force: Boolean = false): String {
        val selected = account(draft.accountId); draft.validate()
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
        if (sent != null) try { ImapClient(selected).use { it.append(bytes,sent.path) } } catch (_: Exception) { return "邮件已发送；已发送文件夹副本保存失败" }
        return "邮件已发送"
    }
    fun reply(message: Message, all: Boolean): Draft {
        val own = account(message.accountId).email.lowercase(Locale.ROOT)
        val recipients = (message.replyTo.ifEmpty { message.from }).filter { it.email.lowercase(Locale.ROOT) != own }
        val seen = recipients.map { it.email.lowercase(Locale.ROOT) }.toMutableSet().apply { add(own) }
        val copies = if (all) (message.to + message.cc).filter { seen.add(it.email.lowercase(Locale.ROOT)) } else emptyList()
        val refs = (message.references + listOf(message.messageId).filter { it.isNotEmpty() }).distinct()
        return Draft(accountId = message.accountId, to = recipients.joinToString(", ") { it.email }, cc = copies.joinToString(", ") { it.email },
            subject = if (message.subject.startsWith("re:",true)) message.subject else "Re: ${message.subject}", text = "\n\n—— ${message.sender}，${Date(message.date)} ——\n${message.body ?: ""}",inReplyTo = message.messageId,references = refs)
    }
    companion object {
        fun friendly(e: Exception): String = when (e) {
            is MailException, is UncertainDelivery -> e.message ?: "邮件操作失败，请重试"
            is javax.net.ssl.SSLException -> "安全连接失败，检查服务器地址与证书"
            is java.net.SocketTimeoutException -> "邮箱连接超时，请重试"
            is java.io.IOException -> "无法连接邮箱，检查网络后重试"
            else -> "邮件操作失败，请检查设置后重试"
        }
    }
}
