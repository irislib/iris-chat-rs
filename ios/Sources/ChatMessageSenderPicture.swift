func irisGroupSenderPictureURL(
    messagePictureURL: String?,
    participant: ChatParticipantSnapshot?
) -> String? {
    // A present participant is current, including a deliberately removed photo.
    if let participant { return participant.pictureUrl }
    return messagePictureURL
}
