package com.jianxin.mail

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/// Private files are authenticated and encrypted with a device-bound Android Keystore key.
class SecureStore(context: Context) {
    private val directory = File(context.noBackupFilesDir, "mail").apply { mkdirs() }
    private fun lock(name: String) = locks.computeIfAbsent(directory.absolutePath + ":" + name) { Any() }
    private val key: SecretKey by lazy {
        synchronized(keyLock) {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey) ?: KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).build())
            generateKey()
        }
        }
    }
    private fun file(name: String) = AtomicFile(File(directory, MessageDigest.getInstance("SHA-256").digest(name.toByteArray()).joinToString("") { "%02x".format(it.toInt() and 255) }))
    fun read(name: String): String? = synchronized(lock(name)) {
        val record = file(name)
        val data = try { record.readFully() } catch (_: java.io.FileNotFoundException) { return@synchronized null }
        if (data.isEmpty()) throw MailException("本地邮箱存储损坏，无法读取")
        val size = data[0].toInt() and 255
        if (size !in 12..16 || data.size <= size + 1) throw MailException("本地邮箱存储损坏，无法读取")
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, data.copyOfRange(1, size + 1)))
            cipher.doFinal(data.copyOfRange(size + 1, data.size)).toString(Charsets.UTF_8)
        } catch (_: Exception) { throw MailException("无法解密本地邮箱数据，请检查设备安全存储") }
    }
    fun write(name: String, value: String) = synchronized(lock(name)) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, key)
        val encrypted = cipher.doFinal(value.toByteArray())
        val bytes = byteArrayOf(cipher.iv.size.toByte()) + cipher.iv + encrypted
        val record = file(name); val output = record.startWrite()
        try { output.write(bytes); record.finishWrite(output) } catch (e: Exception) { record.failWrite(output); throw e }
    }
    fun remove(name: String) = synchronized(lock(name)) { file(name).delete() }
    companion object { private const val ALIAS = "jianxin-mail-v1"; private val locks = java.util.concurrent.ConcurrentHashMap<String,Any>(); private val keyLock = Any() }
}
