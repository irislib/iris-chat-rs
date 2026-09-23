import AVFoundation

/// H.264 access units use Annex B on every platform. Parameter sets accompany
/// every IDR, so a new decoder can recover without previous out-of-band state.
enum IrisH264Wire {
    static let maximumBytes = 262_144
    static let startCode = Data([0, 0, 0, 1])

    static func units(_ data: Data) -> [Data]? {
        guard !data.isEmpty, data.count <= maximumBytes else { return nil }
        let bytes = [UInt8](data)
        var starts: [(Int, Int)] = []
        var offset = 0
        while offset + 2 < bytes.count {
            if bytes[offset] == 0, bytes[offset + 1] == 0 {
                if bytes[offset + 2] == 1 { starts.append((offset, offset + 3)); offset += 3; continue }
                if offset + 3 < bytes.count, bytes[offset + 2] == 0, bytes[offset + 3] == 1 {
                    starts.append((offset, offset + 4)); offset += 4; continue
                }
            }
            offset += 1
        }
        guard starts.first?.0 == 0, starts.count <= 256 else { return nil }
        var result: [Data] = []
        for index in starts.indices {
            let end = index + 1 < starts.count ? starts[index + 1].0 : bytes.count
            guard starts[index].1 < end else { return nil }
            let unit = Data(bytes[starts[index].1..<end])
            guard let first = unit.first, first & 0x80 == 0, first & 0x1f > 0 else { return nil }
            result.append(unit)
        }
        return result
    }

    static func avcc(_ units: [Data]) -> Data {
        var data = Data()
        for unit in units {
            let count = UInt32(unit.count)
            data.append(contentsOf: [UInt8(count >> 24), UInt8((count >> 16) & 255),
                                     UInt8((count >> 8) & 255), UInt8(count & 255)])
            data.append(unit)
        }
        return data
    }

    static func annexB(_ sample: CMSampleBuffer, keyFrame: Bool) -> Data? {
        guard let block = CMSampleBufferGetDataBuffer(sample),
              let format = CMSampleBufferGetFormatDescription(sample) else { return nil }
        var result = Data()
        if keyFrame {
            for index in 0..<2 {
                var pointer: UnsafePointer<UInt8>?
                var size = 0
                guard CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: index,
                    parameterSetPointerOut: &pointer, parameterSetSizeOut: &size,
                    parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == noErr,
                    let pointer, size > 0 else { return nil }
                result.append(startCode)
                result.append(pointer, count: size)
            }
        }
        let length = CMBlockBufferGetDataLength(block)
        guard length > 0, length <= maximumBytes else { return nil }
        var bytes = [UInt8](repeating: 0, count: length)
        guard CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: &bytes) == noErr else { return nil }
        var offset = 0
        while offset + 4 <= length {
            let count = Int(bytes[offset]) << 24 | Int(bytes[offset + 1]) << 16 |
                Int(bytes[offset + 2]) << 8 | Int(bytes[offset + 3])
            offset += 4
            guard count > 0, count <= length - offset else { return nil }
            result.append(startCode)
            result.append(contentsOf: bytes[offset..<(offset + count)])
            offset += count
        }
        return offset == length && result.count <= maximumBytes ? result : nil
    }
}
