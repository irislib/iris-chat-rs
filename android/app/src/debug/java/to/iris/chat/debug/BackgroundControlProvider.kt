package to.iris.chat.debug

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.system.Os
import java.io.File
import java.net.URI
import org.json.JSONObject
import to.iris.chat.BuildConfig

/** Runs before IrisChatApp constructs its core, including ordinary service recreation. */
class BackgroundControlProvider : ContentProvider() {
    override fun onCreate(): Boolean {
        val app = requireNotNull(context)
        if (app.packageName != PACKAGE) return true
        check(BuildConfig.DEBUG && !BuildConfig.SELF_UPDATE_ENABLED)
        val file = File(app.cacheDir, CONFIG_FILE)
        check(file.isFile && file.length() in 1..4096)
        val environment = backgroundControlEnvironment(JSONObject(file.readText()))
        // No defaults from a previous process/launcher may survive the bootstrap phase.
        Os.unsetenv("IRIS_UPDATE_HTREE_REF")
        environment.forEach { (key, value) -> Os.setenv(key, value, true) }
        return true
    }

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?,
        selectionArgs: Array<out String>?, sortOrder: String?): Cursor? = null
    override fun getType(uri: Uri): String? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = 0

    companion object {
        const val PACKAGE = "to.iris.chat.backgroundcontrol"
        const val CONFIG_FILE = "background-control.json"
    }
}

internal fun backgroundControlEnvironment(config: JSONObject): Map<String, String> {
    val phase = config.getString("phase")
    require(phase in setOf("bootstrap", "paired"))
    val allowed = setOf("phase", "relay_url") +
        if (phase == "paired") setOf("peer_npub", "peer_udp", "local_udp_port") else emptySet()
    require(config.keys().asSequence().toSet() == allowed)
    val relay = config.getString("relay_url")
    val url = URI(relay)
    require(url.scheme == "ws" && url.host == "127.0.0.1" && url.port in 1..65535)
    require(url.rawUserInfo == null && url.rawQuery == null && url.rawFragment == null && url.path.isNullOrEmpty())
    val result = linkedMapOf(
        "IRIS_DEMO_RELAYS" to relay,
        "IRIS_FIPS_WEBSOCKET_SEED_URLS" to "",
        "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR" to "",
        "IRIS_CHAT_FIPS_ENABLE_WEBRTC" to "0",
        "IRIS_CHAT_SAME_HOST_HASHTREE" to "0",
        "IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR" to "",
        "IRIS_CHAT_FIPS_ROUTED_PEERS" to "",
        "IRIS_CHAT_FIPS_STATIC_PEERS" to "",
        "IRIS_CHAT_FIPS_UDP_BIND_ADDR" to "",
        "IRIS_RUNTIME_DEBUG_SNAPSHOT" to "0",
        "IRIS_UPDATE_RELAYS" to relay,
        "IRIS_UPDATE_BLOSSOM_SERVERS" to "",
    )
    if (phase == "paired") {
        val npub = config.getString("peer_npub")
        require(Regex("npub1[023456789acdefghjklmnpqrstuvwxyz]{58}").matches(npub))
        val address = config.getString("peer_udp")
        val target = URI("udp://$address")
        val octets = target.host?.split('.')?.map { it.toIntOrNull() } ?: emptyList()
        require(octets.size == 4 && octets.all { it != null && it in 0..255 })
        require(octets[0] == 10 || (octets[0] == 172 && octets[1]!! in 16..31) ||
            (octets[0] == 192 && octets[1] == 168))
        require(target.port in 1..65535 && target.rawUserInfo == null && target.rawQuery == null &&
            target.rawFragment == null && target.path.isNullOrEmpty())
        val port = config.get("local_udp_port")
        require(port is Int && port in 1024..65535)
        result["IRIS_CHAT_FIPS_STATIC_PEERS"] = "$npub=udp:$address"
        result["IRIS_CHAT_FIPS_UDP_BIND_ADDR"] = "0.0.0.0:$port"
        result["IRIS_UPDATE_HTREE_REF"] = "htree://$npub/diagnostic-control/latest"
    }
    return result
}
