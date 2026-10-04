package to.iris.chat.ui.screens

import android.content.ContentResolver
import android.net.Uri
import android.provider.DocumentsContract
import java.io.OutputStream
import java.util.UUID
import to.iris.chat.rust.DirectFileDestination
import to.iris.chat.rust.DirectFileDestinationException
import to.iris.chat.rust.DirectFileSnapshot

/** Worker-thread streaming into exclusively created documents, never an existing file. */
internal class DirectFileDocumentDestination(
    private val resolver: ContentResolver,
    private val tree: Uri,
) : DirectFileDestination {
    private var folder: Uri? = null
    private val documents = mutableListOf<Uri>()
    private val streams = mutableListOf<OutputStream?>()
    private var names: List<String> = emptyList()
    private var committed = false

    private inline fun <T> saving(block: () -> T): T = try { block() } catch (error: Exception) {
        throw DirectFileDestinationException.Failed("Couldn’t save received files")
    }

    @Synchronized
    override fun prepare(transferId: String, files: List<DirectFileSnapshot>) = saving {
        check(folder == null && !committed)
        try {
            val parent = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
            // Create a new batch folder. No URI selected by the user is ever opened for writing.
            val created = requireNotNull(DocumentsContract.createDocument(resolver, parent,
                DocumentsContract.Document.MIME_TYPE_DIR, "Iris files ${UUID.randomUUID()}"))
            folder = created
            names = files.mapIndexed { index, file -> "${index + 1}-${file.filename}" }
            files.forEachIndexed { index, _ ->
                val uri = requireNotNull(DocumentsContract.createDocument(resolver, created,
                    "application/octet-stream", "$index.part"))
                documents.add(uri)
                streams.add(requireNotNull(resolver.openOutputStream(uri, "w")))
            }
        } catch (error: Exception) {
            abort()
            throw error
        }
    }

    @Synchronized
    override fun write(fileIndex: UInt, bytes: ByteArray) = saving {
        check(bytes.size <= 32 * 1024)
        requireNotNull(streams[fileIndex.toInt()]).write(bytes)
    }

    @Synchronized
    override fun finishFile(fileIndex: UInt) = saving {
        val index = fileIndex.toInt()
        val stream = requireNotNull(streams[index])
        stream.flush()
        stream.close()
        streams[index] = null
    }

    @Synchronized
    override fun commit(): List<String> = saving {
        check(!committed && streams.all { it == null })
        documents.indices.forEach { index ->
            documents[index] = requireNotNull(DocumentsContract.renameDocument(resolver, documents[index], names[index]))
        }
        committed = true
        documents.map(Uri::toString)
    }

    @Synchronized
    override fun abort() {
        if (committed) return
        streams.forEach { runCatching { it?.close() } }
        streams.clear()
        documents.forEach { uri -> runCatching { DocumentsContract.deleteDocument(resolver, uri) } }
        documents.clear()
        folder?.let { uri -> runCatching { DocumentsContract.deleteDocument(resolver, uri) } }
        folder = null
    }
}
