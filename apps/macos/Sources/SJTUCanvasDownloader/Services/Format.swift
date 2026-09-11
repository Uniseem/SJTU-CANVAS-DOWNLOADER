import Foundation
import SwiftUI

/// Display text shared by the views.
enum Format {
    private static func formatter(_ pattern: String, timeZone: TimeZone? = nil) -> DateFormatter {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "zh_CN")
        formatter.dateFormat = pattern
        if let timeZone {
            formatter.timeZone = timeZone
        }
        return formatter
    }

    private static let clockFormatter = formatter("HH:mm")
    private static let dayFormatter = formatter("M月d日 HH:mm")
    private static let yearFormatter = formatter("yyyy年M月d日")
    /// The video platform reports lesson times in China time without a zone.
    private static let schoolZone = TimeZone(identifier: "Asia/Shanghai")
    private static let schoolFormats = ["yyyy-MM-dd HH:mm:ss", "yyyy-MM-dd HH:mm", "yyyy-MM-dd'T'HH:mm:ss", "yyyy-MM-dd'T'HH:mm"]
        .map { formatter($0, timeZone: schoolZone) }
    private static let weekdays = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"]

    static func size(_ bytes: Int64) -> String {
        switch bytes {
        case ..<1024:
            return "\(bytes) B"
        case ..<(1024 * 1024):
            return String(format: "%.0f KB", Double(bytes) / 1024)
        case ..<(1024 * 1024 * 1024):
            return String(format: "%.1f MB", Double(bytes) / 1024 / 1024)
        default:
            return String(format: "%.2f GB", Double(bytes) / 1024 / 1024 / 1024)
        }
    }

    static func speed(_ bytesPerSecond: Double) -> String {
        bytesPerSecond <= 0 ? "" : size(Int64(bytesPerSecond)) + "/s"
    }

    /// "剩余 3 分钟" while the speed is known.
    static func remaining(received: Int64, total: Int64?, speed: Double) -> String {
        guard let total, speed >= 1, total > received else { return "" }
        let seconds = Double(total - received) / speed
        switch seconds {
        case ..<60:
            return "剩余 \(max(1, Int(seconds))) 秒"
        case ..<3600:
            return "剩余 \(Int((seconds / 60).rounded(.up))) 分钟"
        default:
            return String(format: "剩余 %.1f 小时", seconds / 3600)
        }
    }

    static func relative(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) {
            return "今天 " + clockFormatter.string(from: date)
        }
        if calendar.isDateInYesterday(date) {
            return "昨天 " + clockFormatter.string(from: date)
        }
        if calendar.isDate(date, equalTo: Date(), toGranularity: .year) {
            return dayFormatter.string(from: date)
        }
        return yearFormatter.string(from: date)
    }

    /// "9月1日 周二 08:00–09:40" from the school's local times.
    static func lessonTime(begin: String, end: String) -> String {
        guard let start = schoolTime(begin) else { return begin }
        var calendar = Calendar(identifier: .gregorian)
        if let schoolZone {
            calendar.timeZone = schoolZone
        }
        let parts = calendar.dateComponents([.year, .month, .day, .weekday, .hour, .minute], from: start)
        let weekday = weekdays[((parts.weekday ?? 1) - 1 + 7) % 7]
        var text = "\(parts.month ?? 0)月\(parts.day ?? 0)日 \(weekday) \(clock(parts))"
        if let finish = schoolTime(end), calendar.isDate(finish, inSameDayAs: start) {
            text += "–" + clock(calendar.dateComponents([.hour, .minute], from: finish))
        }
        let thisYear = calendar.component(.year, from: Date())
        return parts.year == thisYear ? text : "\(parts.year ?? thisYear)年\(text)"
    }

    private static func clock(_ parts: DateComponents) -> String {
        String(format: "%02d:%02d", parts.hour ?? 0, parts.minute ?? 0)
    }

    private static func schoolTime(_ text: String) -> Date? {
        let value = text.trimmed
        for formatter in schoolFormats {
            if let date = formatter.date(from: value) {
                return date
            }
        }
        return nil
    }

    static func trackLabel(_ track: String?) -> String {
        switch track {
        case "slides": return "电脑屏幕"
        case "teacher": return "教室摄像头"
        case "composite": return "合成画面"
        default: return "视频"
        }
    }

    static func trackSymbol(_ track: String) -> String {
        switch track {
        case "slides": return "display"
        case "teacher": return "video"
        default: return "rectangle.split.2x1"
        }
    }

    static func fileSymbol(contentType: String?, name: String) -> String {
        let type = contentType ?? ""
        switch (name as NSString).pathExtension.lowercased() {
        case "pdf": return "doc.richtext"
        case "zip", "rar", "7z": return "doc.zipper"
        case "ppt", "pptx", "key": return "rectangle.on.rectangle"
        case "doc", "docx", "pages", "txt", "md": return "doc.text"
        case "xls", "xlsx", "csv", "numbers": return "tablecells"
        default:
            if type.hasPrefix("video/") { return "film" }
            if type.hasPrefix("image/") { return "photo" }
            if type.hasPrefix("audio/") { return "waveform" }
            return "doc"
        }
    }

    static func status(_ status: String) -> String {
        switch status {
        case "queued": return "排队中"
        case "downloading": return "下载中"
        case "paused": return "已暂停"
        case "completed": return "已完成"
        case "failed": return "失败"
        case "cancelled": return "已取消"
        default: return status
        }
    }

    static func statusSymbol(_ status: String) -> String {
        switch status {
        case "completed": return "checkmark.circle.fill"
        case "failed": return "exclamationmark.triangle.fill"
        case "cancelled": return "xmark.circle.fill"
        case "paused": return "pause.circle.fill"
        case "downloading": return "arrow.down.circle.fill"
        default: return "clock.fill"
        }
    }

    static func statusColor(_ status: String) -> Color {
        switch status {
        case "completed": return .green
        case "failed": return .red
        case "cancelled", "paused": return .secondary
        default: return .accentColor
        }
    }
}

extension String {
    var trimmed: String {
        trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
