import Foundation
import ImageIO
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

func irisRelativeTime(_ secs: UInt64?, relativeTo now: Date = Date()) -> String? {
    guard let secs else { return nil }
    let date = Date(timeIntervalSince1970: TimeInterval(secs))
    let elapsedSeconds = abs(date.timeIntervalSince(now))
    if elapsedSeconds < 60 {
        return "now"
    }
    if elapsedSeconds < 60 * 60 {
        return "\(Int(elapsedSeconds / 60))m"
    }
    if elapsedSeconds < 24 * 60 * 60 {
        return "\(Int(elapsedSeconds / (60 * 60)))h"
    }
    return "\(Int(elapsedSeconds / (24 * 60 * 60)))d"
}

func irisTimelineDay(_ secs: UInt64) -> String {
    let date = Date(timeIntervalSince1970: TimeInterval(secs))
    let calendar = Calendar.current
    if calendar.isDateInToday(date) {
        return "Today"
    }
    if calendar.isDateInYesterday(date) {
        return "Yesterday"
    }
    return irisDayFormatter.string(from: date)
}

func irisMessageClock(_ secs: UInt64) -> String {
    irisTimeFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(secs)))
}

func irisSameTimelineDay(_ lhs: UInt64, _ rhs: UInt64) -> Bool {
    Calendar.current.isDate(
        Date(timeIntervalSince1970: TimeInterval(lhs)),
        inSameDayAs: Date(timeIntervalSince1970: TimeInterval(rhs))
    )
}

func irisDeliveryLabel(_ delivery: DeliveryState) -> String {
    switch delivery {
    case .queued:
        return "Queued"
    case .pending:
        return "Pending"
    case .sent:
        return "Sent"
    case .received:
        return "Received"
    case .seen:
        return "Seen"
    case .failed:
        return "Failed"
    }
}

let irisDayFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateFormat = "EEE, MMM d"
    return formatter
}()

let irisTimeFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .none
    formatter.timeStyle = .short
    return formatter
}()
