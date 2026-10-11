package com.jianxin.mail

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class OAuthTests {
    private val settings = OAuthSettings(clientId = "test-client")
    private fun rejects(work: () -> Unit) { try { work(); fail("invalid OAuth state accepted") } catch (_: MailException) { } }
    private fun start() = MicrosoftOAuth.start(settings) { _,_ -> deviceResponse() }
    private fun deviceResponse() = JSONObject().put("device_code","private-device-code").put("user_code","ABCD-EFGH")
        .put("verification_uri","https://www.microsoft.com/devicelogin").put("expires_in",900).put("interval",5)
    private fun tokenResponse() = JSONObject().put("token_type","Bearer").put("access_token","sample-access-token")
        .put("refresh_token","sample-refresh-token").put("expires_in",3600)
    private fun config(auth: JSONObject) = JSONObject().put("accounts",JSONArray().put(JSONObject().put("email","sample@outlook.com").put("auth",auth)))
    @Test fun importOutlookWithoutDesktopTokensAsPending() {
        val auth = JSONObject().put("type","oauth2").put("client_id",settings.clientId)
            .put("token_endpoint",settings.tokenEndpoint).put("refresh_token",JSONObject().put("file","never-read"))
        for (secret in listOf(null,JSONObject().put("command","never-execute"))) {
            auth.put("access_token",secret)
            val preview = JianxinConfig.parse(config(auth).toString().toByteArray())
            assertEquals(1,preview.accounts.size)
            val account = preview.accounts.single()
            assertTrue(account.needsAuthorization); assertEquals(settings,account.oauthSettings)
            assertEquals("outlook.office365.com",account.imapHost); assertEquals(587,account.smtpPort); assertTrue(account.smtpStartTLS)
            account.validate(); rejects { account.validate(requireCredential = true) }
            assertEquals(account,MailAccount.parse(account.json()))
            assertTrue(preview.warnings.any { it.contains("待授权") })
        }
    }
    @Test fun pendingOAuthDoesNotRelaxPasswordValidation() {
        val preview = JianxinConfig.parse(config(JSONObject().put("type","password")).toString().toByteArray())
        assertTrue(preview.accounts.isEmpty())
        rejects { MailAccount(email="sample@outlook.com",imapHost="outlook.office365.com",smtpHost="smtp-mail.outlook.com").validate() }
        val pending = JianxinConfig.parse(config(JSONObject().put("type","oauth2")).toString().toByteArray()).accounts.single()
        assertTrue(pending.needsAuthorization); assertEquals("",pending.oauthSettings?.clientId)
        rejects { MicrosoftOAuth.start(pending.oauthSettings!!) { _,_ -> fail("should not request without client ID"); JSONObject() } }
    }
    @Test fun importedOAuthScopesAndMicrosoft365TenantPreserved() {
        val endpoint = "https://login.microsoftonline.com/organizations/oauth2/v2.0/token"
        val auth = JSONObject().put("type","o_auth2").put("client_id","client").put("token_endpoint",endpoint)
            .put("scopes",JSONArray(MicrosoftOAuth.DEFAULT_SCOPES))
        val root = config(auth); root.getJSONArray("accounts").getJSONObject(0).put("email","user@company.example")
            .put("imap",JSONObject().put("host","outlook.office365.com").put("port",993))
        val account = JianxinConfig.parse(root.toString().toByteArray()).accounts.single()
        assertEquals(endpoint,account.oauthSettings?.tokenEndpoint); assertEquals(MicrosoftOAuth.DEFAULT_SCOPES,account.oauthSettings?.scopes)
    }
    @Test fun beginRequestsMailAndOfflinePermissions() {
        val session = MicrosoftOAuth.start(settings.copy(scopes = MicrosoftOAuth.DEFAULT_SCOPES.drop(1))) { endpoint,form ->
            assertTrue(endpoint.endsWith("/devicecode")); assertEquals(settings.clientId,form["client_id"])
            assertTrue(form["scope"]!!.contains("offline_access")); assertTrue(form["scope"]!!.contains("IMAP.AccessAsUser.All"))
            deviceResponse()
        }
        assertEquals("ABCD-EFGH",session.userCode)
        assertTrue(session.deadline > System.nanoTime())
    }
    @Test fun pollPendingSlowDownAndSuccess() {
        val session = start()
        assertNull(MicrosoftOAuth.poll(session) { _,form ->
            assertEquals("urn:ietf:params:oauth:grant-type:device_code",form["grant_type"])
            assertEquals("private-device-code",form["device_code"]); JSONObject().put("error","authorization_pending")
        })
        assertNull(MicrosoftOAuth.poll(session) { _,_ -> JSONObject().put("error","slow_down") })
        assertEquals(10L,session.intervalSeconds)
        val tokens = MicrosoftOAuth.poll(session) { _,_ -> tokenResponse() }!!
        assertEquals("sample-access-token",tokens.accessToken); assertEquals("sample-refresh-token",tokens.refreshToken)
        assertTrue(tokens.expiresAt > System.currentTimeMillis() + 3_500_000)
    }
    @Test fun denialExpiryAndInvalidTokenResponsesStopLogin() {
        for (error in listOf("authorization_declined","expired_token","invalid_client","invalid_scope")) rejects {
            MicrosoftOAuth.poll(start()) { _,_ -> JSONObject().put("error",error).put("error_description","private-device-code") }
        }
        val expired = DeviceAuthorization(settings,"device","user","https://microsoft.com/devicelogin",System.nanoTime()-1,5)
        rejects { MicrosoftOAuth.poll(expired) { _,_ -> fail("expired code must not be sent"); JSONObject() } }
        rejects { MicrosoftOAuth.poll(start()) { _,_ -> tokenResponse().put("expires_in",0) } }
        rejects { MicrosoftOAuth.poll(start()) { _,_ -> tokenResponse().put("access_token","") } }
        rejects { MicrosoftOAuth.poll(start()) { _,_ -> tokenResponse().put("access_token",123) } }
        rejects { MicrosoftOAuth.poll(start()) { _,_ -> tokenResponse().apply { remove("refresh_token") } } }
    }
    @Test fun refreshRotatesTokensAndKeepsPreviousWhenNotReturned() {
        val current = settings.copy(refreshToken="old-refresh")
        val next = MicrosoftOAuth.refresh(current) { endpoint,form ->
            assertEquals(settings.tokenEndpoint,endpoint); assertEquals("refresh_token",form["grant_type"])
            assertEquals("old-refresh",form["refresh_token"]); tokenResponse()
        }
        assertEquals("sample-refresh-token",next.refreshToken)
        val kept = MicrosoftOAuth.refresh(current) { _,_ -> tokenResponse().apply { remove("refresh_token") } }
        assertEquals("old-refresh",kept.refreshToken)
        try { MicrosoftOAuth.refresh(current) { _,_ -> JSONObject().put("error","invalid_grant") }; fail("must require reauthorization") }
        catch (_: OAuthLoginRequired) { }
    }
    @Test fun untrustedEndpointsAndBrowserURLsRejected() {
        for (endpoint in listOf("http://login.microsoftonline.com/common/oauth2/v2.0/token","https://evil.example/common/oauth2/v2.0/token",
            "https://login.microsoftonline.com@evil.example/common/oauth2/v2.0/token","https://login.microsoftonline.com/common/oauth2/v2.0/token?redirect=evil",
            "https://login.microsoftonline.com:444/common/oauth2/v2.0/token")) rejects { MicrosoftOAuth.endpoint(endpoint) }
        rejects { MicrosoftOAuth.start(settings) { _,_ -> deviceResponse().put("verification_uri","https://evil.example/login") } }
    }
    @Test fun repeatImportKeepsPhoneAuthorizationAndChangedClientRequiresLogin() {
        val pending = JianxinConfig.parse(config(JSONObject().put("type","oauth2").put("client_id",settings.clientId)).toString().toByteArray()).accounts.single()
        val saved = pending.copy(credential="phone-access",oauthSettings=settings.copy(refreshToken="phone-refresh",expiresAt=123456))
        val merged = MailAccount.merge(listOf(pending),listOf(saved)).single()
        assertEquals(saved.id,merged.id); assertEquals(saved.credential,merged.credential); assertEquals(saved.oauthSettings,merged.oauthSettings)
        assertEquals(merged,MailAccount.parse(merged.json()))
        val changed = pending.copy(oauthSettings=settings.copy(clientId="another-client"))
        assertTrue(MailAccount.merge(listOf(changed),listOf(saved)).single().needsAuthorization)
    }
}
