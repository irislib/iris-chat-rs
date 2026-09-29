#if os(macOS)
import AVFoundation
import AudioToolbox
import CoreAudio
import SwiftUI

struct IrisMacAudioDevice: Identifiable, Equatable {
    let id: String
    let name: String
    let deviceID: AudioDeviceID
}

struct IrisMacAudioDeviceSelection: Equatable {
    var input: AudioDeviceID?
    var output: AudioDeviceID?

    func apply(to engine: AVAudioEngine) throws {
        try apply(write: { isInput, selected in
            var device = selected
            let unit = isInput ? engine.inputNode.audioUnit : engine.outputNode.audioUnit
            guard let unit else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(kAudio_ParamError)) }
            let status = AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice,
                kAudioUnitScope_Global, 0, &device, UInt32(MemoryLayout<AudioDeviceID>.size))
            guard status == noErr else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
        }, read: { isInput in
            let unit = isInput ? engine.inputNode.audioUnit : engine.outputNode.audioUnit
            guard let unit else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(kAudio_ParamError)) }
            var actual: AudioDeviceID = 0
            var size = UInt32(MemoryLayout<AudioDeviceID>.size)
            let status = AudioUnitGetProperty(unit, kAudioOutputUnitProperty_CurrentDevice,
                kAudioUnitScope_Global, 0, &actual, &size)
            guard status == noErr else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
            return actual
        })
    }

    func apply(write: (Bool, AudioDeviceID) throws -> Void, read: (Bool) throws -> AudioDeviceID) throws {
        if let input { try write(true, input) }
        if let output { try write(false, output) }
        // Some hardware couples input and output. Never claim a different
        // device was selected if the second change replaced the first one.
        for (isInput, expected) in [(true, input), (false, output)] {
            guard let expected else { continue }
            guard try read(isInput) == expected else {
                throw NSError(domain: NSOSStatusErrorDomain, code: Int(kAudio_ParamError))
            }
        }
    }
}

struct IrisMacAudioDeviceSnapshot: Equatable {
    var inputs: [IrisMacAudioDevice] = []
    var outputs: [IrisMacAudioDevice] = []
    var defaultInput: AudioDeviceID?
    var defaultOutput: AudioDeviceID?

    func selection(input: String, output: String) -> IrisMacAudioDeviceSelection {
        if input.isEmpty && output.isEmpty { return .init() }
        return .init(input: inputs.first { $0.id == input }?.deviceID ?? defaultInput,
                     output: outputs.first { $0.id == output }?.deviceID ?? defaultOutput)
    }
}

/// Device discovery reads CoreAudio properties without opening a microphone.
/// Stable UIDs are preferences; transient device IDs are resolved each time.
@MainActor
final class IrisMacCallAudioDevices: ObservableObject {
    @Published private(set) var devices = IrisMacAudioDeviceSnapshot()
    @Published private(set) var input = ""
    @Published private(set) var output = ""
    private let defaults: UserDefaults?
    private let apply: (IrisMacAudioDeviceSelection, @escaping (Bool) -> Void) -> Void
    private var monitor: IrisMacAudioDeviceMonitor?
    private var generation = 0
    private var appliedInput = ""
    private var appliedOutput = ""
    private var appliedGeneration = 0

    init(defaults: UserDefaults? = .standard,
         apply: @escaping (IrisMacAudioDeviceSelection, @escaping (Bool) -> Void) -> Void) {
        self.defaults = defaults
        self.apply = apply
        input = defaults?.string(forKey: "callAudioInput") ?? ""
        output = defaults?.string(forKey: "callAudioOutput") ?? ""
    }

    func start() {
        guard monitor == nil else { return }
        monitor = IrisMacAudioDeviceMonitor { [weak self] snapshot in
            Task { @MainActor in self?.update(snapshot) }
        }
    }

    func update(_ snapshot: IrisMacAudioDeviceSnapshot) {
        guard devices != snapshot else { return }
        devices = snapshot
        // Unplugged choices return to the system default immediately. Do not
        // silently steal the route back if that device appears again later.
        if !input.isEmpty && !snapshot.inputs.contains(where: { $0.id == input }) { input = "" }
        if !output.isEmpty && !snapshot.outputs.contains(where: { $0.id == output }) { output = "" }
        applySelection()
    }

    func select(input: String? = nil, output: String? = nil) {
        if let input {
            guard input.isEmpty || devices.inputs.contains(where: { $0.id == input }) else { return }
            self.input = input
        }
        if let output {
            guard output.isEmpty || devices.outputs.contains(where: { $0.id == output }) else { return }
            self.output = output
        }
        applySelection()
    }

    private func applySelection() {
        generation += 1
        let request = generation
        let requestedInput = self.input, requestedOutput = self.output
        apply(devices.selection(input: self.input, output: self.output)) { [weak self] succeeded in
            guard let self else { return }
            if succeeded && request >= self.appliedGeneration {
                self.appliedInput = requestedInput
                self.appliedOutput = requestedOutput
                self.appliedGeneration = request
            }
            guard self.generation == request else { return }
            if !succeeded {
                self.input = self.devices.inputs.contains { $0.id == self.appliedInput } ? self.appliedInput : ""
                self.output = self.devices.outputs.contains { $0.id == self.appliedOutput } ? self.appliedOutput : ""
            }
            self.save()
        }
    }

    private func save() {
        defaults?.set(input, forKey: "callAudioInput")
        defaults?.set(output, forKey: "callAudioOutput")
    }
}

private final class IrisMacAudioDeviceMonitor {
    private let queue = DispatchQueue(label: "iris.call.audio.devices", qos: .utility)
    private let changed: (IrisMacAudioDeviceSnapshot) -> Void
    private let selectors = [kAudioHardwarePropertyDevices, kAudioHardwarePropertyDefaultInputDevice,
                             kAudioHardwarePropertyDefaultOutputDevice]
    private var listener: AudioObjectPropertyListenerBlock?

    init(changed: @escaping (IrisMacAudioDeviceSnapshot) -> Void) {
        self.changed = changed
        let listener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in self?.refresh() }
        self.listener = listener
        for selector in selectors {
            var property = Self.address(selector)
            AudioObjectAddPropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &property, queue, listener)
        }
        queue.async { [weak self] in self?.refresh() }
    }

    deinit {
        guard let listener else { return }
        for selector in selectors {
            var property = Self.address(selector)
            AudioObjectRemovePropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &property, queue, listener)
        }
    }

    private func refresh() {
        let system = AudioObjectID(kAudioObjectSystemObject)
        var property = Self.address(kAudioHardwarePropertyDevices)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(system, &property, 0, nil, &size) == noErr else { return }
        var ids = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(system, &property, 0, nil, &size, &ids) == noErr else { return }
        var snapshot = IrisMacAudioDeviceSnapshot()
        for id in ids {
            guard value(id, kAudioDevicePropertyIsHidden) != 1,
                  value(id, kAudioDevicePropertyDeviceIsAlive) == 1 else { continue }
            guard let uid = string(id, kAudioDevicePropertyDeviceUID), let name = string(id, kAudioObjectPropertyName) else { continue }
            let device = IrisMacAudioDevice(id: uid, name: name, deviceID: id)
            if hasChannels(id, scope: kAudioDevicePropertyScopeInput) { snapshot.inputs.append(device) }
            if hasChannels(id, scope: kAudioDevicePropertyScopeOutput) { snapshot.outputs.append(device) }
        }
        snapshot.inputs.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        snapshot.outputs.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        snapshot.defaultInput = value(system, kAudioHardwarePropertyDefaultInputDevice)
        snapshot.defaultOutput = value(system, kAudioHardwarePropertyDefaultOutputDevice)
        changed(snapshot)
    }

    private func hasChannels(_ device: AudioDeviceID, scope: AudioObjectPropertyScope) -> Bool {
        var property = Self.address(kAudioDevicePropertyStreamConfiguration, scope: scope)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(device, &property, 0, nil, &size) == noErr, size > 0 else { return false }
        let storage = UnsafeMutableRawPointer.allocate(byteCount: Int(size), alignment: MemoryLayout<AudioBufferList>.alignment)
        defer { storage.deallocate() }
        guard AudioObjectGetPropertyData(device, &property, 0, nil, &size, storage) == noErr else { return false }
        return UnsafeMutableAudioBufferListPointer(storage.assumingMemoryBound(to: AudioBufferList.self))
            .contains { $0.mNumberChannels > 0 }
    }

    private func string(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> String? {
        var property = Self.address(selector)
        var result: Unmanaged<CFString>?
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(object, &property, 0, nil, &size, &result) == noErr else { return nil }
        return result?.takeRetainedValue() as String?
    }

    private func value(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> AudioDeviceID? {
        var property = Self.address(selector)
        var result: AudioDeviceID = 0
        var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(object, &property, 0, nil, &size, &result) == noErr, result != 0 else { return nil }
        return result
    }

    private static func address(_ selector: AudioObjectPropertySelector,
                                scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
        .init(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    }
}

struct IrisCallAudioDevicesSheet: View {
    @ObservedObject var devices: IrisMacCallAudioDevices
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("Audio").font(.title2.bold())
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 20) {
                GridRow {
                    Text("Microphone")
                    Picker("Microphone", selection: Binding(get: { devices.input }, set: { devices.select(input: $0) })) {
                        Text("System default").tag("")
                        ForEach(devices.devices.inputs) { Text($0.name).tag($0.id) }
                    }.labelsHidden().accessibilityIdentifier("callMicrophonePicker")
                }
                GridRow {
                    Text("Speaker")
                    Picker("Speaker", selection: Binding(get: { devices.output }, set: { devices.select(output: $0) })) {
                        Text("System default").tag("")
                        ForEach(devices.devices.outputs) { Text($0.name).tag($0.id) }
                    }.labelsHidden().accessibilityIdentifier("callOutputPicker")
                }
            }
            HStack { Spacer(); Button("Done") { dismiss() }.keyboardShortcut(.defaultAction) }
        }
        .padding(24).frame(minWidth: 380)
        .onAppear { devices.start() }
    }
}
#endif
