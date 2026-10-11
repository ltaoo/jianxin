package com.jianxin.mail

data class MailThread(val id: String, val messages: List<Message>) {
    val latest get() = messages.maxWith(compareBy<Message> { it.date }.thenBy { it.id })
    val unread get() = messages.any { !it.isRead }
    val starred get() = messages.any { it.isStarred }
}

object MailThreads {
    fun subject(value: String): String {
        var result = value.trim()
        val prefix = Regex("^(?:re:|aw:|fw:|fwd:|答复:|转发:)\\s*",RegexOption.IGNORE_CASE)
        val tag = Regex("^\\[[^\\s\\[\\]]{1,24}\\](?:\\s+|$)")
        while (true) {
            val next = result.replaceFirst(prefix,"").replaceFirst(tag,"").trim()
            if (next == result) return result
            result = next
        }
    }
    fun threadIndexRoot(value: String): String = try {
        val bytes = java.util.Base64.getDecoder().decode(value.trim())
        if (bytes.size < 16) "" else bytes.take(16).joinToString("") { "%02x".format(it.toInt() and 255) }
    } catch (_: IllegalArgumentException) { "" }
    fun group(messages: List<Message>): List<MailThread> {
        val unique = messages.distinctBy { it.id }
        val parents = mutableMapOf<String,String>()
        fun root(key: String): String {
            var current = key
            while (parents[current] != null && parents[current] != current) current = parents.getValue(current)
            var child = key
            while (parents[child] != null && parents[child] != current) { val next = parents.getValue(child); parents[child] = current; child = next }
            parents.putIfAbsent(current,current); return current
        }
        fun join(a: String, b: String) { val left = root(a); val right = root(b); if (left != right) parents[maxOf(left,right)] = minOf(left,right) }
        val keys = mutableMapOf<String,String>()
        unique.forEach { message ->
            val scope = message.accountId + ":"
            val key = if (message.messageId.isNotEmpty()) scope + "header:" + message.messageId else scope + "message:" + message.id
            keys[message.id] = key; root(key)
            (message.references + Regex("<[^<>]+>").findAll(message.inReplyTo).map { it.value }).filter { it.isNotBlank() }.forEach { join(key,scope + "header:" + it) }
            if (message.threadId.isNotEmpty()) join(key,scope + "native:" + (if (message.threadId.startsWith("imap-thread:")) message.folder + ":" else "") + message.threadId)
            if (message.threadIndexRoot.isNotEmpty()) join(key,scope + "outlook:" + message.threadIndexRoot)
            // Match the web client's final fallback without merging empty subjects.
            val normalized = subject(message.subject)
            if (normalized.isNotEmpty()) join(key,scope + "subject:" + normalized)
        }
        return unique.groupBy { root(keys.getValue(it.id)) }.map { (id,items) -> MailThread(id,items.sortedBy { it.date }) }
            .sortedWith(compareByDescending<MailThread> { it.latest.date }.thenBy { it.id })
    }
}
