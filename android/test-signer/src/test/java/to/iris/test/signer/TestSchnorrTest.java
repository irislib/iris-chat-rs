package to.iris.test.signer;

import org.junit.Test;
import static org.junit.Assert.assertEquals;

public class TestSchnorrTest {
    // Official BIP-340 test-vectors.csv, vectors 0 and 1.
    @Test public void bip340VectorZero() {
        String secret = "0000000000000000000000000000000000000000000000000000000000000003";
        assertEquals("f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9", TestSchnorr.publicKey(secret));
        assertEquals("e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca821525f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0",
            TestSchnorr.sign(secret, new byte[32], new byte[32]));
    }

    @Test public void bip340VectorOne() {
        String secret = "b7e151628aed2a6abf7158809cf4f3c762e7160f38b4da56a784d9045190cfef";
        assertEquals("dff1d77f2a671c5f36183726db2341be58feae1da2deced843240f7b502ba659", TestSchnorr.publicKey(secret));
        byte[] aux = new byte[32]; aux[31] = 1;
        assertEquals("6896bd60eeae296db48a229ff71dfe071bde413e6d43f917dc8dcf8c78de33418906d11ac976abccb20b091292bff4ea897efcb639ea871cfa95f6de339e4b0a",
            TestSchnorr.sign(secret, TestSchnorr.unhex("243f6a8885a308d313198a2e03707344a4093822299f31d0082efa98ec4e6c89"), aux));
    }
}
