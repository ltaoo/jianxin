package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test
import java.time.OffsetDateTime

class MailDateTests {
    private val expected = OffsetDateTime.parse("2026-09-25T09:27:50+08:00").toInstant().toEpochMilli()
    @Test fun missingHeaderUsesServerDateInsteadOfSynchronizationTime() {
        // Observed TokenWorks message: no Date header, valid INTERNALDATE.
        val message = Mime.summary("Subject: Example\r\n\r\n".toByteArray(),"a","INBOX","7","2","25-Sep-2026 09:27:50 +0800")
        assertEquals(expected,message.date)
    }
    @Test fun headerDateWinsOverLaterDeliveryDate() {
        assertEquals(expected,MailDate.resolve("Fri, 25 Sep 2026 09:27:50 +0800 (CST)","11-Oct-2026 23:37:00 +0800"))
        assertEquals(expected,MailDate.header("25 Sep 2026 01:27:50 GMT"))
    }
    @Test fun foldedCommentsMissingWeekdayAndColonOffsets() {
        listOf("Fri 25 Sep 2026 09:27:50 +0800","Friday, 25 Sep 2026 09:27:50 +08:00",
            "25  Sep  2026 09:27:50 +0800 (server (local))","25 Sep 26 09:27:50 +0800").forEach { assertEquals(it,expected,MailDate.header(it)) }
        assertEquals(expected - 50000,MailDate.header("25 Sep 2026 09:27 +0800"))
    }
    @Test fun malformedAndAbsentDatesNeverInventNow() {
        listOf("","invalid","31 Sep 2026 09:27:50 +0800","25 Sep 2026 09:27:50","25 Sep 2026 09:27:50 +0800 garbage").forEach { assertNull(MailDate.header(it)) }
        assertEquals(expected,MailDate.resolve("invalid","25-Sep-2026 09:27:50 +0800"))
        assertEquals(0L,MailDate.resolve("invalid","invalid"))
        assertEquals("时间未知",MailTime.format(0))
    }
    @Test fun obsoleteTimezoneAndTwoDigitYearAreDeterministic() {
        assertEquals(expected,MailDate.header("25 Sep 2026 01:27:50 UTC"))
        assertEquals(expected,MailDate.header("24 Sep 2026 20:27:50 CDT"))
        assertEquals(OffsetDateTime.parse("1999-09-25T09:27:50+08:00").toInstant().toEpochMilli(),MailDate.header("25 Sep 99 09:27:50 +0800"))
    }
    @Test fun legacyCacheRetainsBodyWithoutReusingUnverifiedDate() {
        val original = Message("a","INBOX","7","2",date = expected,body = "正文")
        assertEquals(expected,Message.parse(original.json()).date)
        val legacy = original.json().apply { remove("dateVersion") }
        assertEquals(0L,Message.parse(legacy).date); assertEquals("正文",Message.parse(legacy).body)
    }
}
