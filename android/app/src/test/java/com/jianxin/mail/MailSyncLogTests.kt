package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test

class MailSyncLogTests {
    @Test fun successRecordsCompletionAndPreservesResult() {
        val entries = mutableListOf<MailSyncLog>()
        val before = System.currentTimeMillis()
        val result = MailSyncLog.receive("收信",{ entries.add(it) }) { "received" }
        assertEquals("received",result)
        val entry = entries.single()
        assertTrue(entry.completedAt in before..System.currentTimeMillis())
        assertEquals(MailSyncResult.SUCCESS,entry.result)
        assertEquals(entry,MailSyncLog.parse(entry.json()))
    }
    @Test fun failureRecordsSafeResultAndPreservesException() {
        val failures = listOf(
            OAuthLoginRequired() to MailSyncResult.AUTH_REQUIRED,
            javax.net.ssl.SSLException("private-token") to MailSyncResult.TLS_FAILED,
            java.net.SocketTimeoutException("private-token") to MailSyncResult.TIMED_OUT,
            java.io.IOException("private-token") to MailSyncResult.NETWORK_FAILED,
            MailException("private-token") to MailSyncResult.FAILED
        )
        failures.forEach { (error,expected) ->
            val entries = mutableListOf<MailSyncLog>()
            try { MailSyncLog.receive("收信",{ entries.add(it) }) { throw error }; fail("expected failure") }
            catch (actual: Exception) { assertSame(error,actual) }
            assertEquals(expected,entries.single().result)
            assertFalse(entries.single().json().toString().contains("private-token"))
            assertFalse(entries.single().result.label.contains("private-token"))
        }
    }
    @Test fun cancelledReadsDoNotCreateFailureOrSuccessLogs() {
        val entries = mutableListOf<MailSyncLog>()
        for (throwError in listOf(false,true)) {
            val token = MailCancellation()
            try {
                MailCancellation.run(token) {
                    MailSyncLog.receive("收信",{ entries.add(it) }) {
                        token.cancel()
                        if (throwError) throw java.io.IOException("closed socket")
                        "received"
                    }
                }
                fail("expected cancellation")
            } catch (_: java.io.InterruptedIOException) { }
        }
        try { MailSyncLog.receive("收信",{ entries.add(it) }) { throw java.io.InterruptedIOException("navigation cancelled") } }
        catch (_: java.io.InterruptedIOException) { }
        assertTrue(entries.isEmpty())
    }
}
