package to.iris.chat.push

/** Present only authenticated previews; always retain the payload for core recovery. */
internal suspend fun <Invite : Any> deliverCallPush(
    resolve: suspend () -> Invite?,
    present: suspend (Invite) -> Unit,
    ingest: () -> Unit,
) {
    try {
        resolve()?.let { present(it) }
    } finally {
        ingest()
    }
}
