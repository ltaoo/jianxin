package com.jianxin.mail

import java.io.Closeable
import java.io.InterruptedIOException
import java.util.concurrent.atomic.AtomicBoolean

// Each cancellable read owns its connections, so navigation releases blocked socket reads.
class MailCancellation {
    private val cancelled = AtomicBoolean(false)
    private val resources = mutableSetOf<Closeable>()
    fun check() { if (cancelled.get()) throw InterruptedIOException("Mail read cancelled") }
    fun attach(resource: Closeable) {
        val accepted = synchronized(resources) { if (cancelled.get()) false else resources.add(resource) }
        if (!accepted) { runCatching { resource.close() }; check() }
    }
    fun detach(resource: Closeable) { synchronized(resources) { resources.remove(resource) } }
    fun cancel() {
        if (!cancelled.compareAndSet(false,true)) return
        val closing = synchronized(resources) { resources.toList().also { resources.clear() } }
        closing.forEach { runCatching { it.close() } }
    }
    companion object {
        private val active = ThreadLocal<MailCancellation?>()
        fun current(): MailCancellation? = active.get()
        fun checkCurrent() { current()?.check() }
        fun <T> run(token: MailCancellation?, work: () -> T): T {
            val previous = active.get(); active.set(token)
            try { token?.check(); return work() } finally { active.set(previous) }
        }
    }
}
