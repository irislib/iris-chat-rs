import java.io.ByteArrayInputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.security.PrivateKey;
import java.security.Signature;
import java.security.cert.X509Certificate;
import java.time.Instant;
import java.util.Arrays;
import java.util.Base64;
import java.util.HexFormat;

// Public NIP-C1 proof only. The existing Android key never leaves KeyStore memory.
public class CreateZapstoreC1Proof {
    private static final String PUBLISHER = "7118857500bfa007934147623e9075a72e37a514e605524911d0f6caace41b07";
    private static final String CERTIFICATE = "502a25f79314fa12feb2a517b02d897c58514650cb8cd38c80efc07ee69d1326";

    private static String required(String name) {
        String value = System.getenv(name);
        if (value == null || value.isEmpty()) throw new IllegalArgumentException("Missing " + name);
        return value;
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("Expected fresh public output directory");
        Path output = Path.of(args[0]);
        byte[] encodedStore = Base64.getMimeDecoder().decode(required("IRIS_RELEASE_KEYSTORE_BASE64"));
        char[] storePassword = required("IRIS_RELEASE_KEYSTORE_PASSWORD").toCharArray();
        char[] keyPassword = required("IRIS_RELEASE_KEY_PASSWORD").toCharArray();
        try {
            KeyStore store = KeyStore.getInstance("JKS");
            store.load(new ByteArrayInputStream(encodedStore), storePassword);
            String alias = required("IRIS_RELEASE_KEY_ALIAS");
            X509Certificate certificate = (X509Certificate) store.getCertificate(alias);
            if (certificate == null) throw new IllegalStateException("Certificate missing");
            byte[] der = certificate.getEncoded();
            String hash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(der));
            if (!CERTIFICATE.equals(hash)) throw new IllegalStateException("Certificate differs from immutable APK");
            PrivateKey key = (PrivateKey) store.getKey(alias, keyPassword);
            String algorithm = switch (key.getAlgorithm()) {
                case "RSA" -> "SHA256withRSA";
                case "EC" -> "SHA256withECDSA";
                default -> throw new IllegalStateException("Unsupported certificate algorithm");
            };
            long created = Instant.now().getEpochSecond();
            long expiry = created + 365L * 24 * 60 * 60;
            byte[] message = ("Verifying at " + created + " until " + expiry
                + " that I control the following Nostr public key: " + PUBLISHER).getBytes(StandardCharsets.UTF_8);
            Signature signer = Signature.getInstance(algorithm);
            signer.initSign(key);
            signer.update(message);
            byte[] signature = signer.sign();
            Signature verifier = Signature.getInstance(algorithm);
            verifier.initVerify(certificate.getPublicKey());
            verifier.update(message);
            if (!verifier.verify(signature)) throw new IllegalStateException("Certificate challenge verification failed");
            String event = "{\"kind\":30509,\"pubkey\":\"" + PUBLISHER + "\",\"created_at\":" + created
                + ",\"tags\":[[\"d\",\"" + hash + "\"],[\"signature\",\""
                + Base64.getEncoder().encodeToString(signature) + "\"],[\"expiry\",\"" + expiry
                + "\"]],\"content\":\"\"}\n";
            Files.createDirectory(output);
            Files.write(output.resolve("certificate.der"), der);
            Files.writeString(output.resolve("unsigned-c1.json"), event, StandardCharsets.UTF_8);
            System.out.println("Verified public certificate challenge; unsigned Nostr proof ready.");
        } finally {
            Arrays.fill(encodedStore, (byte) 0);
            Arrays.fill(storePassword, '\0');
            Arrays.fill(keyPassword, '\0');
        }
    }
}
