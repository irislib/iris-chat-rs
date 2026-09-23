package to.iris.test.signer;

import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;

/**
 * Dependency-free BIP-340 fixture signer, ONLY for this debug test APK.
 * This variable-time BigInteger implementation is unsuitable for real keys.
 * Production Iris validates its output with the existing Rust Nostr library.
 */
final class TestSchnorr {
    private static final BigInteger P = hexInt("fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f");
    private static final BigInteger N = hexInt("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141");
    private static final Point G = new Point(
        hexInt("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"),
        hexInt("483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"));

    private record Point(BigInteger x, BigInteger y) {}

    static String newSecret() {
        byte[] bytes = new byte[32];
        SecureRandom random = new SecureRandom();
        BigInteger value;
        do { random.nextBytes(bytes); value = new BigInteger(1, bytes); }
        while (value.signum() == 0 || value.compareTo(N) >= 0);
        return hex(bytes);
    }

    static String publicKey(String secret) {
        return hex(bytes32(multiply(secretScalar(secret)).x));
    }

    static String sign(String secret, byte[] message, byte[] aux) {
        BigInteger d = secretScalar(secret);
        Point point = multiply(d);
        if (point.y.testBit(0)) d = N.subtract(d);
        byte[] t = bytes32(d);
        byte[] auxHash = taggedHash("BIP0340/aux", aux);
        for (int i = 0; i < t.length; i++) t[i] ^= auxHash[i];
        BigInteger k = new BigInteger(1, taggedHash("BIP0340/nonce", t, bytes32(point.x), message)).mod(N);
        if (k.signum() == 0) throw new IllegalStateException("Zero nonce");
        Point r = multiply(k);
        if (r.y.testBit(0)) k = N.subtract(k);
        BigInteger e = new BigInteger(1, taggedHash("BIP0340/challenge", bytes32(r.x), bytes32(point.x), message)).mod(N);
        return hex(bytes32(r.x)) + hex(bytes32(k.add(e.multiply(d)).mod(N)));
    }

    static byte[] sha256(byte[] data) {
        try { return MessageDigest.getInstance("SHA-256").digest(data); }
        catch (Exception error) { throw new IllegalStateException(error); }
    }

    private static byte[] taggedHash(String tag, byte[]... inputs) {
        byte[] tagHash = sha256(tag.getBytes(StandardCharsets.UTF_8));
        int length = 64;
        for (byte[] input : inputs) length += input.length;
        byte[] bytes = new byte[length];
        System.arraycopy(tagHash, 0, bytes, 0, 32);
        System.arraycopy(tagHash, 0, bytes, 32, 32);
        int offset = 64;
        for (byte[] input : inputs) {
            System.arraycopy(input, 0, bytes, offset, input.length);
            offset += input.length;
        }
        return sha256(bytes);
    }

    private static BigInteger secretScalar(String secret) {
        BigInteger scalar = hexInt(secret);
        if (scalar.signum() <= 0 || scalar.compareTo(N) >= 0) throw new IllegalArgumentException("Invalid key");
        return scalar;
    }

    private static Point multiply(BigInteger scalar) {
        Point result = null;
        Point addend = G;
        for (int i = 0; i < scalar.bitLength(); i++) {
            if (scalar.testBit(i)) result = add(result, addend);
            addend = add(addend, addend);
        }
        return result;
    }

    private static Point add(Point a, Point b) {
        if (a == null) return b;
        if (b == null) return a;
        BigInteger slope;
        if (a.x.equals(b.x)) {
            if (!a.y.equals(b.y) || a.y.signum() == 0) return null;
            slope = a.x.multiply(a.x).multiply(BigInteger.valueOf(3))
                .multiply(a.y.shiftLeft(1).modInverse(P)).mod(P);
        } else {
            slope = b.y.subtract(a.y).multiply(b.x.subtract(a.x).mod(P).modInverse(P)).mod(P);
        }
        BigInteger x = slope.multiply(slope).subtract(a.x).subtract(b.x).mod(P);
        return new Point(x, slope.multiply(a.x.subtract(x)).subtract(a.y).mod(P));
    }

    private static BigInteger hexInt(String hex) { return new BigInteger(hex, 16); }

    private static byte[] bytes32(BigInteger number) {
        byte[] raw = number.toByteArray();
        byte[] result = new byte[32];
        int count = Math.min(raw.length, 32);
        System.arraycopy(raw, raw.length - count, result, 32 - count, count);
        return result;
    }

    static byte[] unhex(String hex) {
        byte[] bytes = new byte[hex.length() / 2];
        for (int i = 0; i < bytes.length; i++) bytes[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
        return bytes;
    }

    static String hex(byte[] bytes) {
        StringBuilder text = new StringBuilder(bytes.length * 2);
        for (byte value : bytes) text.append(String.format("%02x", value & 255));
        return text.toString();
    }
}
