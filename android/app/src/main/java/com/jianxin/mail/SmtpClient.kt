package com.jianxin.mail

import java.util.Base64
import java.util.Locale

private class SmtpRejection : Exception()

object SmtpClient {
    fun send(draft: Draft, account: MailAccount, connect: () -> MailWire = { MailSocket(account.smtpHost, account.smtpPort, !account.smtpStartTLS) }): ByteArray {
        val message = Mime.compose(draft, account)
        if (message.size > MailSocket.MAX_MESSAGE) throw MailException("邮件超过 25 MB")
        connect().use { wire ->
            fun response(vararg expected: Int, final: Boolean = false): String {
                val lines = mutableListOf<String>()
                repeat(100) {
                    val line = wire.readLine(); lines.add(line)
                    val code = line.take(3).toIntOrNull() ?: throw MailException("SMTP 响应无效")
                    if (line.length < 4 || line[3] != '-') {
                        if (code !in expected) { if (final) throw SmtpRejection(); throw MailException(if (code == 535 || code == 534) "发信登录失败，检查授权码或 OAuth 令牌" else "服务器拒绝发送，检查收件人或邮箱权限") }
                        return lines.joinToString("\n")
                    }
                }
                throw MailException("SMTP 响应过长")
            }
            response(220); wire.line("EHLO jianxin.local"); var ehlo = response(250)
            if (account.smtpStartTLS) {
                if (!ehlo.uppercase(Locale.ROOT).contains("STARTTLS")) throw MailException("SMTP 服务器不支持 STARTTLS")
                wire.line("STARTTLS"); response(220); wire.startTLS(); wire.line("EHLO jianxin.local"); ehlo = response(250)
            }
            if (account.oauth2) {
                val auth = Base64.getEncoder().encodeToString("user=${account.smtpLogin}\u0001auth=Bearer ${account.credential}\u0001\u0001".toByteArray())
                wire.line("AUTH XOAUTH2 $auth"); response(235)
            } else if (ehlo.uppercase(Locale.ROOT).contains("PLAIN")) {
                val auth = Base64.getEncoder().encodeToString("\u0000${account.smtpLogin}\u0000${account.credential}".toByteArray())
                wire.line("AUTH PLAIN $auth"); if (response(235, 334).startsWith("334")) { wire.line(auth); response(235) }
            } else {
                wire.line("AUTH LOGIN"); response(334); wire.line(Base64.getEncoder().encodeToString(account.smtpLogin.toByteArray())); response(334)
                wire.line(Base64.getEncoder().encodeToString(account.credential.toByteArray())); response(235)
            }
            wire.line("MAIL FROM:<${account.email}>"); response(250)
            draft.recipients.map { it.email }.distinct().forEach { wire.line("RCPT TO:<$it>"); response(250, 251) }
            wire.line("DATA"); response(354)
            try { wire.write(dotStuff(message)); wire.line("."); response(250, final = true) }
            catch (_: SmtpRejection) { throw MailException("服务器明确拒绝此邮件，请检查邮箱限制后重试") }
            catch (_: Exception) { throw UncertainDelivery() }
            try { wire.line("QUIT") } catch (_: Exception) { }
        }
        return message
    }
    fun dotStuff(bytes: ByteArray): ByteArray {
        val escaped = bytes.toString(Charsets.UTF_8).split("\r\n").joinToString("\r\n") { if (it.startsWith('.')) ".$it" else it }
        return (if (escaped.endsWith("\r\n")) escaped else "$escaped\r\n").toByteArray()
    }
}
