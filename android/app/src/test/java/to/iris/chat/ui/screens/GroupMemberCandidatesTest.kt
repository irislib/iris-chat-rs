package to.iris.chat.ui.screens

import org.junit.Assert.*
import org.junit.Test
import to.iris.chat.rust.*

class GroupMemberCandidatesTest {
    @Test fun allEligibleContactsRemainAvailableAcrossSequentialAdds() {
        val people = (0..20).map(::person)
        var members = setOf(people[0].chatId)
        val first = groupMemberCandidates(people, people[20].chatId, members, "")
        assertEquals(19, first.size)
        assertEquals(people[19], first.last())
        members = members + people[15].chatId
        val second = groupMemberCandidates(people, people[20].chatId, members, "")
        assertEquals(18, second.size)
        assertFalse(second.any { it.chatId == people[15].chatId })
        assertTrue(second.any { it.chatId == people[19].chatId })
        members = members + people[19].chatId
        assertEquals(17, groupMemberCandidates(people, people[20].chatId, members, "").size)
    }

    @Test fun groupRowsAreExcludedAndPrivateAvatarSnapshotIsPreserved() {
        val favorite = person(1).copy(socialConnection = SocialConnectionSnapshot(null, null, 0u, "Favorite", true))
        val result = groupMemberCandidates(listOf(favorite, person(2).copy(kind = ChatKind.GROUP)), null, emptySet(), "")
        assertEquals(listOf(favorite), result)
        assertTrue(result.single().socialConnection!!.isFavorite)
    }

    private fun person(index: Int) = ChatThreadSnapshot(
        socialConnection = null, chatId = index.toString(16).padStart(64, '0'), kind = ChatKind.DIRECT,
        displayName = "Person $index", nickname = null, contactNote = null, profileName = null,
        subtitle = null, pictureUrl = null, about = null, memberCount = 2u,
        lastMessagePreview = null, lastMessageAtSecs = null, lastMessageIsOutgoing = null,
        lastMessageDelivery = null, unreadCount = 0u, isTyping = false, isMuted = false,
        isPinned = false, draft = "", isRequest = false,
    )
}
