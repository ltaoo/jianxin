package com.jianxin.mail

import java.text.ParsePosition
import java.text.SimpleDateFormat
import java.util.Locale
import java.util.TimeZone

// Parse absolute mail dates; never substitute the time at which a message was fetched.
object MailDate {
    fun header(value: String): Long? {
        var date = value.trim()
        // RFC comments may be nested. Limit the input and work before parsing.
        if (date.length > 512) return null
        repeat(8) { date = date.replace(Regex("\\([^()]*\\)")," ") }
        date = date.replace(Regex("^[A-Za-z]{3,9},?\\s+"),"").replace(Regex("\\s+")," ").trim()
        date = date.replace(Regex("([+-]\\d{2}):(\\d{2})$"),"$1$2")
        // Obsolete RFC 5322 zone names have fixed offsets, independent of device locale.
        val zones = mapOf("UT" to "+0000","UTC" to "+0000","GMT" to "+0000","EST" to "-0500","EDT" to "-0400",
            "CST" to "-0600","CDT" to "-0500","MST" to "-0700","MDT" to "-0600","PST" to "-0800","PDT" to "-0700")
        val zone = date.substringAfterLast(' ').uppercase(Locale.ROOT)
        zones[zone]?.let { date = date.substringBeforeLast(' ') + " " + it }
        // A numeric zone is required: an absent zone must not depend on the phone's timezone.
        if (!Regex("[+-]\\d{4}$").containsMatchIn(date)) return null
        date = Regex("^(\\d{1,2} [A-Za-z]{3} )(\\d{2})( )").replace(date) { match ->
            val year = match.groupValues[2].toInt()
            match.groupValues[1] + (if (year < 50) 2000 + year else 1900 + year) + match.groupValues[3]
        }
        for (pattern in listOf("d MMM yyyy HH:mm:ss Z","d MMM yyyy HH:mm Z")) {
            parse(date,pattern)?.let { return it }
        }
        return null
    }
    fun internal(value: String): Long? = parse(value.trim(),"d-MMM-yyyy HH:mm:ss Z")
    fun resolve(headerValue: String, internalValue: String): Long = header(headerValue) ?: internal(internalValue) ?: 0L
    private fun parse(value: String, pattern: String): Long? {
        val parser = SimpleDateFormat(pattern,Locale.US).apply {
            isLenient = false; timeZone = TimeZone.getTimeZone("UTC")
        }
        val position = ParsePosition(0)
        return parser.parse(value,position)?.takeIf { position.index == value.length }?.time
    }
}
