package com.terraforme.remote.data

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

/**
 * As máquinas com terra que o app já achou, no SQLite do celular: o endereço
 * (o do Tailscale primeiro) e a última lista de instâncias de cada uma.
 */
class HostStore(context: Context) : SQLiteOpenHelper(context, "terra.db", null, 1) {

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            "CREATE TABLE hosts (url TEXT PRIMARY KEY, tailscale INTEGER NOT NULL, " +
                "seen INTEGER NOT NULL, instances TEXT)"
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, old: Int, new: Int) {}

    /** Os endereços salvos: os do Tailscale primeiro, depois o visto por último. */
    fun urls(): List<String> = readableDatabase
        .rawQuery("SELECT url FROM hosts ORDER BY tailscale DESC, seen DESC", null)
        .use { c -> List(c.count) { c.moveToNext(); c.getString(0) } }

    /** Salva `url` como visto agora; com `status`, guarda a lista de instâncias dela. */
    fun save(url: String, status: String? = null) {
        val values = ContentValues().apply {
            put("url", url)
            put("tailscale", if (isTailscale(url)) 1 else 0)
            put("seen", System.currentTimeMillis())
            if (status != null) put("instances", status)
        }
        val db = writableDatabase
        if (db.update("hosts", values, "url = ?", arrayOf(url)) == 0) db.insert("hosts", null, values)
    }

    /** A última lista de instâncias (o JSON do /api/status) salva para `url`. */
    fun instances(url: String): String? = readableDatabase
        .rawQuery("SELECT instances FROM hosts WHERE url = ?", arrayOf(url))
        .use { c -> if (c.moveToFirst() && !c.isNull(0)) c.getString(0) else null }
}

/** Um IP do Tailscale está em 100.64.0.0/10. */
fun isTailscale(url: String): Boolean {
    val octets = url.substringAfter("://").substringBefore(':').split('.').mapNotNull { it.toIntOrNull() }
    return octets.size == 4 && octets[0] == 100 && octets[1] in 64..127
}
