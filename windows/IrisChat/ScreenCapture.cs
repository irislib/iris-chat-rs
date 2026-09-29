using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using Windows.Graphics;
using Windows.Graphics.Capture;
using Windows.Graphics.DirectX;
using Windows.Graphics.DirectX.Direct3D11;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace IrisChat;

// OS-owned picker and capture indicator. Only the selected surface is read;
// screen audio is deliberately not captured (the call microphone is unchanged).
internal sealed class ScreenCapture : IDisposable
{
    private readonly GraphicsCaptureItem _item;
    private readonly IDirect3DDevice _device;
    private readonly Direct3D11CaptureFramePool _pool;
    private readonly GraphicsCaptureSession _session;
    private readonly Action<uint, uint, byte[]> _frame;
    private readonly Action _closed;
    private readonly object _lifecycle = new();
    private SizeInt32 _size;
    private int _disposed;
    private int _reading;
    private long _lastFrame;
    public string Name => _item.DisplayName;

    internal static async Task<GraphicsCaptureItem?> PickAsync(IntPtr window)
    {
        if (!GraphicsCaptureSession.IsSupported()) throw new InvalidOperationException("Screen sharing is unavailable on this device.");
        var picker = new GraphicsCapturePicker();
        WinRT.Interop.InitializeWithWindow.Initialize(picker, window);
        return await picker.PickSingleItemAsync();
    }

    internal ScreenCapture(GraphicsCaptureItem item, Action<uint, uint, byte[]> frame, Action closed)
    {
        _item = item; _frame = frame; _closed = closed; _size = item.Size;
        _device = CreateDevice();
        try
        {
            _pool = Direct3D11CaptureFramePool.CreateFreeThreaded(_device, DirectXPixelFormat.B8G8R8A8UIntNormalized, 2, _size);
            _session = _pool.CreateCaptureSession(item);
            _pool.FrameArrived += OnFrame;
            _item.Closed += OnClosed;
        }
        catch { _device.Dispose(); throw; }
    }

    internal void Start() => _session.StartCapture();
    private void OnClosed(GraphicsCaptureItem sender, object args)
    {
        if (Volatile.Read(ref _disposed) == 0) _closed();
    }

    private async void OnFrame(Direct3D11CaptureFramePool sender, object args)
    {
        if (Volatile.Read(ref _disposed) != 0 || Interlocked.Exchange(ref _reading, 1) != 0) return;
        try
        {
            var size = _size;
            using (var frame = sender.TryGetNextFrame())
            {
                if (frame == null) return;
                size = frame.ContentSize;
                var now = Stopwatch.GetTimestamp();
                if (now - _lastFrame < Stopwatch.Frequency / 15 || size.Width <= 0 || size.Height <= 0) return;
                _lastFrame = now;
                using var bitmap = await SoftwareBitmap.CreateCopyFromSurfaceAsync(frame.Surface, BitmapAlphaMode.Ignore);
                if (Volatile.Read(ref _disposed) != 0) return;
                var width = bitmap.PixelWidth; var height = bitmap.PixelHeight;
                if (width > 8192 || height > 8192 || (long)width * height > 16_777_216) return;
                var bytes = new byte[checked(width * height * 4)];
                var buffer = new Windows.Storage.Streams.Buffer((uint)bytes.Length);
                bitmap.CopyToBuffer(buffer);
                using var reader = DataReader.FromBuffer(buffer);
                reader.ReadBytes(bytes);
                for (var i = 0; i < bytes.Length; i += 4) (bytes[i], bytes[i + 2]) = (bytes[i + 2], bytes[i]);
                // Disposal waits for an in-flight submission. A frame from an
                // old selection cannot arrive after a new selection starts.
                lock (_lifecycle)
                    if (Volatile.Read(ref _disposed) == 0) _frame((uint)width, (uint)height, bytes);
            }
            if (Volatile.Read(ref _disposed) == 0 && (size.Width != _size.Width || size.Height != _size.Height))
            {
                _size = size;
                sender.Recreate(_device, DirectXPixelFormat.B8G8R8A8UIntNormalized, 2, size);
            }
        }
        catch (Exception) { if (Volatile.Read(ref _disposed) == 0) _closed(); }
        finally { Volatile.Write(ref _reading, 0); }
    }

    public void Dispose()
    {
        lock (_lifecycle)
            if (Interlocked.Exchange(ref _disposed, 1) != 0) return;
        _item.Closed -= OnClosed;
        _pool.FrameArrived -= OnFrame;
        _session.Dispose(); _pool.Dispose(); _device.Dispose();
    }

    private static IDirect3DDevice CreateDevice()
    {
        IntPtr device = IntPtr.Zero, context = IntPtr.Zero, dxgi = IntPtr.Zero, inspectable = IntPtr.Zero;
        try
        {
            // D3D11_CREATE_DEVICE_BGRA_SUPPORT, hardware driver. No third-party graphics wrapper.
            Marshal.ThrowExceptionForHR(D3D11CreateDevice(IntPtr.Zero, 1, IntPtr.Zero, 0x20, IntPtr.Zero, 0, 7,
                out device, out _, out context));
            var iid = new Guid("54ec77fa-1377-44e6-8c32-88fd5f44c84c");
            Marshal.ThrowExceptionForHR(Marshal.QueryInterface(device, ref iid, out dxgi));
            Marshal.ThrowExceptionForHR(CreateDirect3D11DeviceFromDXGIDevice(dxgi, out inspectable));
            return WinRT.MarshalInterface<IDirect3DDevice>.FromAbi(inspectable);
        }
        finally
        {
            if (inspectable != IntPtr.Zero) Marshal.Release(inspectable);
            if (dxgi != IntPtr.Zero) Marshal.Release(dxgi);
            if (context != IntPtr.Zero) Marshal.Release(context);
            if (device != IntPtr.Zero) Marshal.Release(device);
        }
    }
    [DllImport("d3d11.dll", ExactSpelling = true)]
    private static extern int D3D11CreateDevice(IntPtr adapter, int driver, IntPtr software, uint flags,
        IntPtr levels, uint count, uint sdk, out IntPtr device, out int level, out IntPtr context);
    [DllImport("d3d11.dll", ExactSpelling = true)]
    private static extern int CreateDirect3D11DeviceFromDXGIDevice(IntPtr device, out IntPtr result);
}
