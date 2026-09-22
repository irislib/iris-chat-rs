#if os(macOS)
import CoreAudio
import Foundation

// Observe device activity without opening an audio stream or accessing audio.
final class MacBluetoothAudioActivity {
    private(set) var isActive = false
    var onChange: (() -> Void)?
    private var devices: [AudioDeviceID] = []
    private var started = false
    private var idleTransition: DispatchWorkItem?
    private lazy var deviceListener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
        self?.refreshDevices()
    }
    private lazy var activityListener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
        self?.refreshActivity()
    }

    func start() {
        guard !started else { return }
        started = true
        var property = address(kAudioHardwarePropertyDevices)
        AudioObjectAddPropertyListenerBlock(
            AudioObjectID(kAudioObjectSystemObject), &property, .main, deviceListener
        )
        refreshDevices()
    }

    func stop() {
        guard started else { return }
        started = false
        idleTransition?.cancel()
        idleTransition = nil
        var property = address(kAudioHardwarePropertyDevices)
        AudioObjectRemovePropertyListenerBlock(
            AudioObjectID(kAudioObjectSystemObject), &property, .main, deviceListener
        )
        removeActivityListeners()
    }

    deinit { stop() }

    private func refreshDevices() {
        guard started else { return }
        let system = AudioObjectID(kAudioObjectSystemObject)
        var property = address(kAudioHardwarePropertyDevices)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(system, &property, 0, nil, &size) == noErr else { return }
        var found = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(system, &property, 0, nil, &size, &found) == noErr else { return }
        removeActivityListeners()
        devices = found.filter {
            let transport = value($0, kAudioDevicePropertyTransportType)
            return transport == kAudioDeviceTransportTypeBluetooth || transport == kAudioDeviceTransportTypeBluetoothLE
        }
        property = address(kAudioDevicePropertyDeviceIsRunningSomewhere)
        for device in devices {
            AudioObjectAddPropertyListenerBlock(device, &property, .main, activityListener)
        }
        refreshActivity()
    }

    private func removeActivityListeners() {
        var property = address(kAudioDevicePropertyDeviceIsRunningSomewhere)
        for device in devices {
            AudioObjectRemovePropertyListenerBlock(device, &property, .main, activityListener)
        }
        devices.removeAll()
    }

    private func refreshActivity() {
        guard started else { return }
        let active = devices.contains { value($0, kAudioDevicePropertyDeviceIsRunningSomewhere) == 1 }
        if active {
            idleTransition?.cancel()
            idleTransition = nil
            guard !isActive else { return }
            isActive = true
            onChange?()
        } else if isActive, idleTransition == nil {
            // Brief pauses and audio route changes must not start a new probe
            // just as playback resumes. Require five seconds of idle audio.
            let idle = DispatchWorkItem { [weak self] in
                guard let self, self.started else { return }
                self.idleTransition = nil
                self.isActive = false
                self.onChange?()
            }
            idleTransition = idle
            DispatchQueue.main.asyncAfter(deadline: .now() + 5, execute: idle)
        }
    }

    private func value(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> UInt32? {
        var property = address(selector)
        var result: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        guard AudioObjectGetPropertyData(object, &property, 0, nil, &size, &result) == noErr else { return nil }
        return result
    }

    private func address(_ selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(
            mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain
        )
    }
}
#endif
