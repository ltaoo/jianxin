package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test

class UnifiedInboxTests {
    private fun mail(account: String, uid: Int, time: Long) = Message(account,"INBOX","7",uid.toString(),date=time)
    @Test fun mergeSortsMultipleAccountsWithoutMergingSameUID() {
        val pager = UnifiedInbox(listOf("a","b"))
        val result = pager.next { id,_ -> InboxBatch(listOf(mail(id,1,if (id == "a") 100 else 200)),false) }
        assertEquals(listOf("b","a"),result.messages.map { it.accountId }); assertFalse(result.hasMore)
        assertTrue(result.errors.isEmpty()); assertEquals(2,result.messages.size)
    }
    @Test fun paginationKeepsIndependentOffsetsAndDeduplicates() {
        val pager = UnifiedInbox(listOf("a","b"))
        val calls = mutableListOf<Pair<String,Int>>()
        val load: (String,Int) -> InboxBatch = { id,offset ->
            calls.add(id to offset)
            InboxBatch(if (offset == 0) listOf(mail(id,1,300)) else listOf(mail(id,1,300),mail(id,2,100)),id == "a" && offset == 0)
        }
        assertTrue(pager.next(load).hasMore)
        val second = pager.next(load)
        assertEquals(listOf("a" to 0,"b" to 0,"a" to 30),calls)
        assertEquals(3,second.messages.size); assertFalse(second.hasMore)
        pager.next(load); assertEquals(3,calls.size)
    }
    @Test fun accountFailureKeepsOtherMailAndItsOwnOfflineCache() {
        val saved = mail("b",3,50)
        val pager = UnifiedInbox(listOf("a","b","pending")) { id -> if (id == "b") listOf(saved) else emptyList() }
        val result = pager.next { id,_ -> when (id) {
            "a" -> InboxBatch(listOf(mail(id,1,100)),false)
            "b" -> throw java.io.IOException("synthetic network failure")
            else -> throw OAuthLoginRequired()
        } }
        assertEquals(listOf("a","b"),result.messages.map { it.accountId })
        assertTrue(result.errors["b"]!!.contains("网络")); assertTrue(result.errors["pending"]!!.contains("授权"))
    }
    @Test fun flagUpdatesAndMovesStayWithinSourceAccount() {
        val pager = UnifiedInbox(listOf("a","b"))
        pager.next { id,_ -> InboxBatch(listOf(mail(id,1,100)),false) }
        val changed = mail("a",1,100).copy(isRead=true,isStarred=true)
        pager.update(changed)
        val updated = pager.next { _,_ -> fail("no more pages"); InboxBatch(emptyList(),false) }
        assertTrue(updated.messages.single { it.accountId == "a" }.isStarred)
        assertFalse(updated.messages.single { it.accountId == "b" }.isRead)
        pager.remove(changed)
        assertEquals("b",pager.next { _,_ -> InboxBatch(emptyList(),false) }.messages.single().accountId)
    }
}
