// nodemind-helper — Nodemind's bridge to macOS frameworks that have no clean
// Rust bindings. Spawned by the app as a sidecar; one job per invocation.
//
//   nodemind-helper calendar <YYYY-MM-DD>
//       Today's events via EventKit, as a JSON array on stdout:
//       [{"id","title","start","end","calendar","allDay"}]  (RFC 3339, local offset)
//
//   nodemind-helper tap
//       System audio output (the other side of a call) via a Core Audio
//       process tap (macOS 14.2+). Binary on stdout: the sample rate as a
//       u32 LE, then mono f32 LE samples until stdin closes or SIGTERM.
//
// Exit codes: 0 ok · 1 error · 2 permission denied · 3 unsupported macOS.
// Messages for the user go to stderr, one line.
//
// Permissions are attributed to the Nodemind app, which carries the usage
// strings (NSCalendarsFullAccessUsageDescription, NSAudioCaptureUsageDescription).

import AudioToolbox
import CoreAudio
import EventKit
import Foundation

func fail(_ message: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

// MARK: - calendar

func calendarDay(_ day: String) {
    let parser = DateFormatter()
    parser.locale = Locale(identifier: "en_US_POSIX")
    parser.timeZone = .current
    parser.dateFormat = "yyyy-MM-dd"
    guard let start = parser.date(from: day),
          let end = Calendar.current.date(byAdding: .day, value: 1, to: start)
    else { fail("bad day '\(day)', expected YYYY-MM-DD") }

    let store = EKEventStore()
    let done = DispatchSemaphore(value: 0)
    var granted = false
    if #available(macOS 14.0, *) {
        store.requestFullAccessToEvents { ok, _ in granted = ok; done.signal() }
    } else {
        store.requestAccess(to: .event) { ok, _ in granted = ok; done.signal() }
    }
    done.wait()
    guard granted else {
        fail("Calendar access is off — allow Nodemind in System Settings → Privacy & Security → Calendars.", code: 2)
    }

    let iso = ISO8601DateFormatter()
    iso.formatOptions = [.withInternetDateTime]
    iso.timeZone = .current
    let predicate = store.predicateForEvents(withStart: start, end: end, calendars: nil)
    let events: [[String: Any]] = store.events(matching: predicate)
        .sorted { $0.startDate < $1.startDate }
        .map { e in
            [
                "id": e.eventIdentifier ?? UUID().uuidString,
                "title": e.title ?? "",
                "start": iso.string(from: e.startDate),
                "end": iso.string(from: e.endDate),
                "calendar": e.calendar?.title ?? "",
                "allDay": e.isAllDay,
            ]
        }
    do {
        FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: events))
    } catch {
        fail("cannot encode events: \(error)")
    }
}

// MARK: - system audio tap

func check(_ status: OSStatus, _ what: String, code: Int32 = 1) {
    if status != noErr { fail("\(what) failed (OSStatus \(status))", code: code) }
}

func defaultOutputUID() -> String {
    var device = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    var addr = AudioObjectPropertyAddress(
        mSelector: kAudioHardwarePropertyDefaultSystemOutputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
    check(AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &addr, 0, nil, &size, &device),
          "reading the default output device")
    var uid: Unmanaged<CFString>?
    size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
    addr.mSelector = kAudioDevicePropertyDeviceUID
    check(AudioObjectGetPropertyData(device, &addr, 0, nil, &size, &uid), "reading the output device id")
    guard let value = uid?.takeRetainedValue() else { fail("output device has no id") }
    return value as String
}

@available(macOS 14.2, *)
func systemAudioTap() {
    // Tap everything the Mac plays (call audio included), without muting it.
    let description = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
    description.uuid = UUID()
    description.name = "Nodemind meeting notes"
    description.isPrivate = true
    description.muteBehavior = .unmuted
    var tap = AudioObjectID(kAudioObjectUnknown)
    check(AudioHardwareCreateProcessTap(description, &tap), "creating the system audio tap", code: 2)

    // A tap is read through a private aggregate device that contains it.
    let outputUID = defaultOutputUID()
    let aggregate: [String: Any] = [
        kAudioAggregateDeviceNameKey: "Nodemind Meeting Tap",
        kAudioAggregateDeviceUIDKey: UUID().uuidString,
        kAudioAggregateDeviceMainSubDeviceKey: outputUID,
        kAudioAggregateDeviceIsPrivateKey: true,
        kAudioAggregateDeviceIsStackedKey: false,
        kAudioAggregateDeviceTapAutoStartKey: true,
        kAudioAggregateDeviceSubDeviceListKey: [[kAudioSubDeviceUIDKey: outputUID]],
        kAudioAggregateDeviceTapListKey: [
            [kAudioSubTapDriftCompensationKey: true, kAudioSubTapUIDKey: description.uuid.uuidString],
        ],
    ]
    var device = AudioObjectID(kAudioObjectUnknown)
    check(AudioHardwareCreateAggregateDevice(aggregate as CFDictionary, &device), "creating the capture device")

    var format = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    var addr = AudioObjectPropertyAddress(
        mSelector: kAudioTapPropertyFormat,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
    check(AudioObjectGetPropertyData(tap, &addr, 0, nil, &size, &format), "reading the tap format")
    guard format.mFormatFlags & kAudioFormatFlagIsFloat != 0, format.mBitsPerChannel == 32 else {
        fail("unexpected tap format (flags \(format.mFormatFlags), \(format.mBitsPerChannel) bit)")
    }

    let out = FileHandle.standardOutput
    var rate = UInt32(format.mSampleRate).littleEndian
    out.write(Data(bytes: &rate, count: 4))

    var proc: AudioDeviceIOProcID?
    let queue = DispatchQueue(label: "nodemind.tap", qos: .userInteractive)
    check(AudioDeviceCreateIOProcIDWithBlock(&proc, device, queue) { _, input, _, _, _ in
        let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
        guard let first = buffers.first, first.mData != nil else { return }
        var mono: [Float]
        if buffers.count == 1 {
            // Interleaved: average the channels of each frame.
            let channels = max(1, Int(first.mNumberChannels))
            let samples = first.mData!.assumingMemoryBound(to: Float.self)
            let frames = Int(first.mDataByteSize) / 4 / channels
            mono = [Float](repeating: 0, count: frames)
            for f in 0..<frames {
                var sum: Float = 0
                for c in 0..<channels { sum += samples[f * channels + c] }
                mono[f] = sum / Float(channels)
            }
        } else {
            // Non-interleaved: one buffer per channel.
            let frames = Int(first.mDataByteSize) / 4
            mono = [Float](repeating: 0, count: frames)
            for b in buffers {
                guard let data = b.mData else { continue }
                let ch = data.assumingMemoryBound(to: Float.self)
                for f in 0..<frames { mono[f] += ch[f] / Float(buffers.count) }
            }
        }
        mono.withUnsafeBytes { out.write(Data($0)) }
    }, "starting system audio capture")
    // The first start is what shows the "record system audio" permission prompt.
    check(AudioDeviceStart(device, proc), "starting system audio capture", code: 2)

    let stop: () -> Never = {
        AudioDeviceStop(device, proc)
        if let proc { AudioDeviceDestroyIOProcID(device, proc) }
        AudioHardwareDestroyAggregateDevice(device)
        AudioHardwareDestroyProcessTap(tap)
        exit(0)
    }
    // Stop when the app closes our stdin (or exits), or on SIGTERM.
    DispatchQueue.global().async {
        _ = FileHandle.standardInput.readDataToEndOfFile()
        stop()
    }
    signal(SIGTERM, SIG_IGN)
    let term = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
    term.setEventHandler { stop() }
    term.resume()
    dispatchMain()
}

// MARK: - entry

let args = CommandLine.arguments
switch args.count > 1 ? args[1] : "" {
case "calendar":
    guard args.count > 2 else { fail("usage: nodemind-helper calendar YYYY-MM-DD") }
    calendarDay(args[2])
case "tap":
    if #available(macOS 14.2, *) {
        systemAudioTap()
    } else {
        fail("Meeting audio capture needs macOS 14.2 or newer.", code: 3)
    }
default:
    fail("usage: nodemind-helper calendar YYYY-MM-DD | tap")
}
