package com.jianxin.mail

import org.json.JSONObject

enum class MailSyncResult(val label: String) {
    SUCCESS("成功"), AUTH_REQUIRED("待授权，请先登录邮箱"), TLS_FAILED("安全连接失败，检查服务器地址与证书"),
    TIMED_OUT("连接超时，请重试"), NETWORK_FAILED("连接失败，检查网络后重试"), FAILED("失败，请检查邮箱设置后重试");

    companion object {
        fun failure(error: Exception) = when (error) {
            is OAuthLoginRequired -> AUTH_REQUIRED
            is javax.net.ssl.SSLException -> TLS_FAILED
            is java.net.SocketTimeoutException -> TIMED_OUT
            is java.io.IOException -> NETWORK_FAILED
            else -> FAILED
        }
    }
}

data class MailSyncLog(val completedAt: Long, val operation: String, val result: MailSyncResult) {
    fun json() = JSONObject().put("completedAt",completedAt).put("operation",operation).put("result",result.name)
    companion object {
        const val RETENTION = 1000
        fun parse(json: JSONObject) = MailSyncLog(json.getLong("completedAt"),json.getString("operation"),MailSyncResult.valueOf(json.getString("result")))

        fun <T> receive(operation: String, record: (MailSyncLog) -> Unit, work: () -> T): T {
            MailCancellation.checkCurrent()
            val value = try { work().also { MailCancellation.checkCurrent() } } catch (error: Exception) {
                MailCancellation.checkCurrent()
                if (error is InterruptedException || Thread.currentThread().isInterrupted ||
                    (error is java.io.InterruptedIOException && error !is java.net.SocketTimeoutException)) throw error
                record(MailSyncLog(System.currentTimeMillis(),operation,MailSyncResult.failure(error)))
                throw error
            }
            record(MailSyncLog(System.currentTimeMillis(),operation,MailSyncResult.SUCCESS))
            return value
        }
    }
}
