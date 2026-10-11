package com.jianxin.mail

import java.io.InputStream
import java.io.ByteArrayOutputStream

object ConfigFile {
    // Do not trust a document provider's reported size; bound the actual bytes read.
    fun read(stream: InputStream, password: String, method: ConfigCipher): ConfigPreview {
        val out = ByteArrayOutputStream()
        val buffer = ByteArray(8192)
        while (true) {
            val count = stream.read(buffer)
            if (count < 0) break
            if (out.size() + count > ConfigEncryption.LIMIT) throw MailException("加密配置不能超过 2 MB")
            out.write(buffer,0,count)
        }
        val plaintext = ConfigEncryption.decrypt(out.toByteArray(),password,method)
        return try { JianxinConfig.parse(plaintext) } finally { plaintext.fill(0) }
    }
}
