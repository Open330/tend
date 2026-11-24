import Foundation

enum TendMenuBarPolling {
    static func interval(pointerInside: Bool, animating: Bool, popoverShown: Bool, waitingToFold: Bool = false,
                         distanceToMenuBar: CGFloat = 0, pointerEvents: Bool = false) -> TimeInterval {
        // Keep animation/tracking smooth; idle hover detection needs at most
        // 67ms, rather than waking the run loop sixty times every second.
        if animating || waitingToFold || (pointerInside && !popoverShown) { return 1.0 / 60 }
        // Near a menu bar, mouse-moved events drive the checks; the timer
        // only notices a move the events missed (and runs the periodic
        // refresh), after which polling resumes at the full rate.
        if pointerEvents, distanceToMenuBar < eventDrivenDistance { return eventCheck }
        return idle(distanceToMenuBar: distanceToMenuBar)
    }

    /// Within this distance of a menu bar the pointer is followed by its
    /// mouse-moved events; farther out the slow idle timer is cheaper than
    /// an event for every move.
    static let eventDrivenDistance: CGFloat = 450
    /// How often the timer still checks while events drive the hover.
    static let eventCheck: TimeInterval = 0.25
    /// The fastest the pointer is checked while events drive it: the idle
    /// rate near a menu bar, as when polling.
    static let eventSpacing: TimeInterval = 1.0 / 15

    /// A pointer far from every menu bar is checked less often; near one,
    /// at 15 Hz as before. Each interval is what a fast flick (about 3000
    /// points a second) needs to cover the distance: a guide for keeping
    /// hover responsive, not a guarantee (the timer has a quarter-interval
    /// tolerance, and a warped pointer jumps).
    static func idle(distanceToMenuBar distance: CGFloat) -> TimeInterval {
        switch distance {
        case 1500...: 0.5
        case 900...: 0.3
        case 450...: 0.15
        default: 1.0 / 15
        }
    }

    static func distance(from point: CGPoint, to bands: [CGRect]) -> CGFloat {
        bands.map { band in
            let dx = max(band.minX - point.x, 0, point.x - band.maxX)
            let dy = max(band.minY - point.y, 0, point.y - band.maxY)
            return (dx * dx + dy * dy).squareRoot()
        }.min() ?? 0
    }

    static func refreshUsage(chipShown: Bool, lastAttempt: Date, lastSuccess: Date?, now: Date) -> Bool {
        chipShown && now.timeIntervalSince(max(lastAttempt, lastSuccess ?? .distantPast)) >= 120
    }
}

/// Mouse-moved events in place of the 15 Hz poll near a menu bar. An event
/// is checked at once unless the last check was under `eventSpacing` ago;
/// then one trailing check follows, so the pointer is never checked more
/// often than the poll did and its final position is never skipped. The
/// events are not guaranteed (a warped pointer sends none, and some apps'
/// tracking swallows them): a move the timer finds without an event turns
/// polling back on until events arrive again. Times are monotonic
/// (`ProcessInfo.systemUptime`), so a wall-clock change cannot stall a check.
struct TendMenuBarPointerEvents {
    private(set) var installed = false
    /// Bumped on every install and removal; a trailing check queued for an
    /// earlier installation is dropped.
    private(set) var generation: UInt64 = 0
    private(set) var lastEventAt: TimeInterval?
    private(set) var lastCheckAt = -TimeInterval.infinity
    private(set) var trailingPending = false
    /// The timer saw the pointer move without an event.
    private(set) var missed = false

    /// Events can be relied on to report the next move.
    var drivesHover: Bool { installed && !missed }

    enum Check: Equatable {
        case now
        /// Check after this delay, if `generation` is still current then.
        case after(TimeInterval, generation: UInt64)
        /// A trailing check is already scheduled, or the timer is tracking.
        case none
    }

    mutating func install() {
        installed = true
        reset()
    }

    mutating func uninstall() {
        installed = false
        reset()
    }

    private mutating func reset() {
        generation &+= 1
        trailingPending = false
        missed = false
        lastEventAt = nil
        lastCheckAt = -.infinity
    }

    /// An event arrived. `timerTracking`: the fast timer follows the
    /// pointer anyway (a hover, an animation), so no extra check is needed.
    mutating func received(at now: TimeInterval, timerTracking: Bool) -> Check {
        guard installed else { return .none }
        lastEventAt = now
        missed = false
        guard !timerTracking, !trailingPending else { return .none }
        let wait = TendMenuBarPolling.eventSpacing - (now - lastCheckAt)
        guard wait > 0 else {
            lastCheckAt = now
            return .now
        }
        trailingPending = true
        return .after(wait, generation: generation)
    }

    /// The trailing check is due; false when it belongs to an earlier
    /// installation and must not run.
    mutating func trailingCheck(at now: TimeInterval, generation queued: UInt64) -> Bool {
        guard installed, queued == generation else { return false }
        trailingPending = false
        lastCheckAt = now
        return true
    }

    /// The timer found the pointer somewhere it was not at the last check.
    /// Allow for an event that is queued behind the timer.
    mutating func timerSawMove(at now: TimeInterval) {
        guard installed, !trailingPending else { return }
        if let lastEventAt, now - lastEventAt < 0.1 { return }
        missed = true
    }
}

@MainActor
struct TendMenuBarRenderState: Equatable {
    let chips: [TendMenuBarChip]
    let positions: [CGFloat]
    let focus: Int?
    let pillWidth: CGFloat
    let lead: CGFloat
    let markOpacity: CGFloat
    let overflow: Int
    let selectedID: String?
    let liveIDs: Set<String>
    let dark: Bool
    let mark: TendBrandMark
    let backingScales: [CGFloat]
    private let pixels: [TendMenuBarChipPixels]

    init(chips: [TendMenuBarChip], positions: [CGFloat], focus: Int?, pillWidth: CGFloat,
         lead: CGFloat, markOpacity: CGFloat, overflow: Int, selectedID: String?,
         liveIDs: Set<String>, dark: Bool, mark: TendBrandMark, backingScales: [CGFloat] = [1]) {
        self.chips = chips
        self.positions = positions
        self.focus = focus
        self.pillWidth = pillWidth
        self.lead = lead
        self.markOpacity = markOpacity
        self.overflow = overflow
        self.selectedID = selectedID
        self.liveIDs = liveIDs
        self.dark = dark
        self.mark = mark
        self.backingScales = backingScales
        pixels = chips.enumerated().map { index, chip in
            let badge = chip.status.needsAttention || chip.unread ? TendMenuBarBadgeStyle(status: chip.status) : nil
            return TendMenuBarChipPixels(palette: chip.appearance.paletteIndex, symbol: chip.appearance.symbol,
                          text: index == focus && pillWidth > TendMenuBarGeometry.disc * 1.1 ? chip.pillLabel : nil,
                          badge: badge, live: liveIDs.contains(chip.id), selected: selectedID == chip.id)
        }
    }
    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        guard lhs.positions == rhs.positions, lhs.focus == rhs.focus,
              lhs.pillWidth == rhs.pillWidth, lhs.lead == rhs.lead,
              lhs.markOpacity == rhs.markOpacity, lhs.overflow == rhs.overflow,
              lhs.dark == rhs.dark, lhs.mark == rhs.mark,
              lhs.backingScales == rhs.backingScales else { return false }
        return lhs.pixels == rhs.pixels
    }
}

private struct TendMenuBarChipPixels: Equatable, Sendable {
    let palette: Int
    let symbol: String
    let text: String?
    let badge: TendMenuBarBadgeStyle?
    let live: Bool
    let selected: Bool
}

struct TendMenuBarAccessibilityState: Equatable {
    let ids: [String]
    let labels: [String]
    let rects: [CGRect]
    let overflow: Int
}

/// Screen-space hover intent. A narrow margin preserves a capsule while the
/// pointer travels toward its peek; movement within one exit zone never keeps
/// postponing the fold indefinitely.
enum TendMenuBarHoverPolicy {
    enum Zone: Equatable { case item, margin, peek, outside }
    static let exitGrace: TimeInterval = 0.2
    static let peekGrace: TimeInterval = 0.45
    static let unfoldDuration: TimeInterval = 0.16
    static let foldDuration: TimeInterval = 0.26

    static func zone(point: CGPoint, item: CGRect, peek: CGRect?) -> Zone {
        if item.contains(point) { return .item }
        let margin = CGRect(x: item.minX - 4, y: item.minY - 10,
                            width: item.width + 8, height: item.height + 11)
        if margin.contains(point) { return .margin }
        if let peek {
            let bridge = CGRect(x: peek.minX, y: peek.minY, width: peek.width,
                                height: max(0, item.minY - peek.minY))
            if bridge.contains(point) { return .peek }
        }
        return .outside
    }
}

struct TendMenuBarFoldIntent {
    private(set) var deadline: Date?
    private var grace: TimeInterval?
    mutating func cancel() { deadline = nil; grace = nil }
    mutating func schedule(now: Date, grace: TimeInterval) {
        guard deadline == nil || self.grace != grace else { return }
        self.grace = grace
        deadline = now.addingTimeInterval(grace)
    }
    mutating func consume(now: Date) -> Bool {
        guard let deadline, now >= deadline else { return false }
        cancel()
        return true
    }
}

/// A transient card can close on mouse-down, before the status item's mouse-up.
/// Suppress only that same destination; another chip must open immediately.
enum TendMenuBarCardInteraction {
    enum Action { case open, close, ignore }
    static func action(requested: String?, current: String?, shown: Bool,
                       closed: String?, closedAt: Date?, now: Date) -> Action {
        if shown { return requested == current ? .close : .open }
        if let closedAt, now.timeIntervalSince(closedAt) < 0.6, requested == closed { return .ignore }
        return .open
    }
}
