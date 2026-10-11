package com.jianxin.mail

object MailProviders {
    private val platforms = mapOf("gmail" to "gmail","google" to "gmail","outlook" to "outlook","microsoft" to "outlook",
        "qq" to "qq","foxmail" to "foxmail","163" to "netease163","netease163" to "netease163","126" to "netease126","netease126" to "netease126",
        "yeah" to "yeah","aliyun" to "aliyun","sina" to "sina","139" to "mail139","mail139" to "mail139","icloud" to "icloud",
        "zoho" to "zoho","yahoo" to "yahoo","fastmail" to "fastmail","proton" to "proton","tencent" to "tencent","exmail" to "tencent")
    fun key(account: MailAccount): String {
        val explicit = account.platform.trim().ifEmpty { account.provider.trim() }.lowercase(java.util.Locale.ROOT)
        if (explicit.isNotEmpty()) return platforms[explicit] ?: ""
        // Accounts stored by older Android versions do not have presentation metadata.
        val domain = account.email.substringAfterLast('@').lowercase(java.util.Locale.ROOT)
        val name = when (domain) { "googlemail.com" -> "google"; "hotmail.com","live.com","office365.com" -> "outlook"; "me.com" -> "icloud"; "proton.me","protonmail.com" -> "proton"; else -> domain.substringBefore('.') }
        return platforms[name] ?: ""
    }
}
