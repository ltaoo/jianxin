package com.jianxin.mail

import java.util.Locale
import java.util.Date

object MailReplies {
    fun draft(message: Message, ownAddress: String, all: Boolean): Draft {
        val own = ownAddress.lowercase(Locale.ROOT)
        val sentBySelf = message.from.any { it.email.equals(own,true) }
        val primary = if (sentBySelf && message.replyTo.isEmpty()) message.to else message.replyTo.ifEmpty { message.from }
        val seen = mutableSetOf(own)
        val recipients = primary.filter { seen.add(it.email.lowercase(Locale.ROOT)) }
        val copies = if (all) (message.to + message.cc).filter { seen.add(it.email.lowercase(Locale.ROOT)) } else emptyList()
        val refs = (message.references + listOf(message.messageId).filter { it.isNotEmpty() }).distinct().takeLast(32)
        return Draft(accountId = message.accountId,to = recipients.joinToString(", ") { it.email },cc = copies.joinToString(", ") { it.email },
            subject = if (message.subject.startsWith("re:",true)) message.subject else "Re: ${message.subject}",
            text = "\n\n—— ${message.sender}，${Date(message.date)} ——\n${message.body ?: ""}",inReplyTo = message.messageId,references = refs)
    }
}
