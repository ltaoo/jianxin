package com.jianxin.mail

data class InboxBatch(val messages: List<Message>, val hasMore: Boolean)
data class InboxResult(val messages: List<Message>, val hasMore: Boolean, val errors: Map<String,String>)

// Each account keeps its own IMAP offset. A failed account cannot discard successful mail.
class UnifiedInbox(private val accountIds: List<String>, private val cached: (String) -> List<Message> = { emptyList() }) {
    private val offsets = mutableMapOf<String,Int>()
    private val mail = mutableMapOf<String,List<Message>>()
    private val more = accountIds.associateWith { true }.toMutableMap()
    private val errors = mutableMapOf<String,String>()
    fun next(load: (String,Int) -> InboxBatch): InboxResult = next({ false },load)
    fun next(cancelled: () -> Boolean, load: (String,Int) -> InboxBatch): InboxResult {
        for (id in accountIds) {
            if (cancelled()) break
            if (more[id] != true) continue
            try {
                val batch = load(id,offsets[id] ?: 0)
                mail[id] = ((mail[id] ?: emptyList()) + batch.messages).distinctBy { it.id }
                offsets[id] = (offsets[id] ?: 0) + 30; more[id] = batch.hasMore; errors.remove(id)
            } catch (e: Exception) {
                if (mail[id].isNullOrEmpty()) mail[id] = runCatching { cached(id) }.getOrDefault(emptyList())
                errors[id] = MailErrors.friendly(e); more[id] = false
            }
        }
        return InboxResult(sort(mail.values.flatten()),more.values.any { it },errors.toMap())
    }
    fun update(message: Message) { mail[message.accountId] = mail[message.accountId]?.map { if (it.id == message.id) message else it } ?: emptyList() }
    fun remove(message: Message) { mail[message.accountId] = mail[message.accountId]?.filter { it.id != message.id } ?: emptyList() }
    companion object {
        fun sort(messages: List<Message>) = messages.distinctBy { it.id }.sortedWith(compareByDescending<Message> { it.date }.thenBy { it.id })
    }
}

object MailErrors {
    fun friendly(e: Exception): String = when (e) {
        is MailException, is UncertainDelivery -> e.message ?: "邮件操作失败，请重试"
        is javax.net.ssl.SSLException -> "安全连接失败，检查服务器地址与证书"
        is java.net.SocketTimeoutException -> "邮箱连接超时，请重试"
        is java.io.IOException -> "无法连接邮箱，检查网络后重试"
        else -> "邮件操作失败，请检查设置后重试"
    }
}
