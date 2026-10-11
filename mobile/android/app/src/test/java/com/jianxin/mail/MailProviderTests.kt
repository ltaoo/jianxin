package com.jianxin.mail

import org.junit.Assert.*
import org.junit.Test

class MailProviderTests {
    @Test fun providerSelectionFillsConnectionAndLogoBeforeEmail() {
        val a = MailAccount().apply { preset("QQ") }
        assertEquals("qq",MailProviders.key(a)); assertEquals("imap.qq.com",a.imapHost); assertEquals(993,a.imapPort)
        a.preset("Outlook"); assertTrue(a.needsAuthorization); assertEquals("outlook",MailProviders.key(a)); assertTrue(a.smtpStartTLS)
        a.preset("Gmail"); assertFalse(a.oauth2); assertEquals("gmail",MailProviders.key(a))
    }
    @Test fun platformOverridesDomainAndProviderAsOnWeb() {
        val a = MailAccount(email = "a@qq.com",platform = " MICROSOFT ",provider = "google")
        assertEquals("outlook",MailProviders.key(a)); assertEquals("outlook",MailProviders.key(MailAccount.parse(a.json())))
        assertEquals("gmail",MailProviders.key(a.copy(platform = "")))
        assertEquals("",MailProviders.key(a.copy(platform = "custom")))
    }
    @Test fun legacyAccountsHaveDomainFallback() {
        assertEquals("qq",MailProviders.key(MailAccount(email = "a@qq.com")))
        assertEquals("outlook",MailProviders.key(MailAccount(email = "a@hotmail.com")))
        assertEquals("proton",MailProviders.key(MailAccount(email = "a@proton.me")))
        assertEquals("",MailProviders.key(MailAccount(email = "a@example.com")))
    }
    @Test fun providerMetadataSurvivesEncryptedConfigImport() {
        val json = """{"accounts":[{"id":"provider","email":"a@example.com","platform":"qq","provider":"imap","auth":{"type":"password","password":"synthetic"},"imap":{"host":"imap.qq.com","port":993},"smtp":{"host":"smtp.qq.com","port":465}}]}"""
        val account = JianxinConfig.parse(json.toByteArray()).accounts.single()
        assertEquals("qq",account.platform); assertEquals("imap",account.provider)
    }
}
