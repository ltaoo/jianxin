package com.jianxin.mail

import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

object MailTime {
    const val DEFAULT = "24h"
    const val TWELVE = "12h"
    fun validate(style: String) {
        if (style == DEFAULT || style == TWELVE) return
        if (style.isBlank() || style.length > 80 || style.any { it == '\n' || it == '\r' }) throw MailException("请输入有效时间格式，例如 yyyy-MM-dd HH:mm")
        try { SimpleDateFormat(style,Locale.getDefault()) } catch (_: IllegalArgumentException) { throw MailException("时间格式无效，小时请使用 HH 或 hh") }
    }
    fun format(timestamp: Long, style: String = DEFAULT, compact: Boolean = false, now: Long = System.currentTimeMillis()): String {
        validate(style)
        if (timestamp == 0L) return "时间未知"
        val pattern = if (style == DEFAULT || style == TWELVE) {
            val time = if (style == DEFAULT) "HH:mm" else "hh:mm a"
            val day = Calendar.getInstance().apply { timeInMillis = timestamp }
            val today = Calendar.getInstance().apply { timeInMillis = now }
            val sameDay = day.get(Calendar.YEAR) == today.get(Calendar.YEAR) && day.get(Calendar.DAY_OF_YEAR) == today.get(Calendar.DAY_OF_YEAR)
            if (!compact) "yyyy-MM-dd $time" else if (sameDay) time else "MM-dd $time"
        } else style
        return SimpleDateFormat(pattern,Locale.getDefault()).format(Date(timestamp))
    }
}
