package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test
import java.util.Calendar

class MailTimeTests {
    private fun timestamp(year: Int = 2026, month: Int = 9, day: Int = 11) = Calendar.getInstance().apply { clear(); set(year,month,day,13,45,30) }.timeInMillis
    @Test fun defaultUses24HourClockForListAndReader() {
        val time = timestamp()
        assertEquals("2026-10-11 13:45",MailTime.format(time))
        assertEquals("13:45",MailTime.format(time,compact = true,now = time))
        assertEquals("10-11 13:45",MailTime.format(time,compact = true,now = timestamp(day = 12)))
        assertEquals("10-11 13:45",MailTime.format(time,compact = true,now = timestamp(year = 2027)))
    }
    @Test fun twelveHourAndCustomFormatsApply() {
        assertTrue(MailTime.format(timestamp(),MailTime.TWELVE).contains("01:45"))
        assertEquals("11/10/2026 13:45:30",MailTime.format(timestamp(),"dd/MM/yyyy HH:mm:ss"))
    }
    @Test fun invalidCustomPatternsAreRejected() {
        listOf(""," ","yyyy-MM-dd QQ:mm","HH:mm\nss","'unfinished","H".repeat(81)).forEach { style ->
            try { MailTime.validate(style); fail("Invalid format accepted") } catch (_: MailException) { }
        }
    }
}
