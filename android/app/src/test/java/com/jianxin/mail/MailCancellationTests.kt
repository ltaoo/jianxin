package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test
import java.io.Closeable
import java.io.InterruptedIOException
import java.net.ServerSocket
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

class MailCancellationTests {
    @Test fun cancelledBeforeConnectionCannotLeakOrStartWork() {
        val token = MailCancellation(); val closed = AtomicInteger(); token.cancel()
        try { token.attach(Closeable { closed.incrementAndGet() }); fail("cancelled connection accepted") } catch (_: InterruptedIOException) { }
        assertEquals(1,closed.get())
        try { MailCancellation.run(token) { fail("cancelled work started") }; fail("cancel ignored") } catch (_: InterruptedIOException) { }
    }
    @Test fun releasedConnectionsAreNotClosedByLaterNavigation() {
        val token = MailCancellation(); val closed = AtomicInteger(); val resource = Closeable { closed.incrementAndGet() }
        token.attach(resource); token.detach(resource); token.cancel(); token.cancel()
        assertEquals(0,closed.get())
    }
    @Test fun cancelledReadContextCannotCancelAnUnrelatedMutation() {
        val token = MailCancellation()
        MailCancellation.run(token) {
            token.cancel()
            MailCancellation.run(null) { assertNull(MailCancellation.current()); MailCancellation.checkCurrent() }
            try { MailCancellation.checkCurrent(); fail("read resumed") } catch (_: InterruptedIOException) { }
        }
        assertNull(MailCancellation.current())
    }
    @Test fun navigationUnblocksAStalledSocketWithoutWaitingForTimeout() {
        ServerSocket(0).use { server ->
            val pool = Executors.newSingleThreadExecutor(); val reading = CountDownLatch(1); val token = MailCancellation()
            try {
                val task = pool.submit<Boolean> {
                    try {
                        MailCancellation.run(token) { MailSocket("127.0.0.1",server.localPort,false).use { reading.countDown(); it.readLine() } }
                        false
                    } catch (_: Exception) { true }
                }
                server.accept().use {
                    assertTrue(reading.await(2,TimeUnit.SECONDS)); token.cancel()
                    assertTrue(task.get(2,TimeUnit.SECONDS))
                }
            } finally { token.cancel(); pool.shutdownNow() }
        }
    }
}
