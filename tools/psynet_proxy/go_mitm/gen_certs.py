"""Generate the VelocityRL root CA (once) and per-host psynet leaf certs.

Config MITM trust path (the only supported setup):
  - Install velocityrl_ca.crt into LocalMachine\\Root (Schannel)
  - hosts: config.psynet.gg -> 127.0.0.1
  - Do NOT overwrite C:\\Program Files\\Common Files\\SSL\\cert.pem,
    C:\\Windows\\cert.pem, game cacert.pem, or plant a singleton CA into
    OpenSSL CAPATH dirs — that broke EAC/EOS TLS in the past.

openssl_trust/ is a local artifact for debugging only.
For WS MITM OpenSSL trust use install_openssl_trust.ps1 (APPEND into
Mozilla cert.pem). Never copy openssl_trust/cert.pem system-wide.
"""

from __future__ import annotations

import datetime
import hashlib
import ipaddress
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

HERE = Path(__file__).parent
CA_CERT = HERE / "velocityrl_ca.crt"
CA_KEY = HERE / "velocityrl_ca.key"
CRL_PATH = HERE / "velocityrl.crl"

HOSTS = [
    "api.rlpp.psynet.gg",
    "ws.rlpp.psynet.gg",
    "config.psynet.gg",
]

CA_NAME = x509.Name(
    [
        x509.NameAttribute(NameOID.COMMON_NAME, "VelocityRL"),
        x509.NameAttribute(NameOID.ORGANIZATION_NAME, "VelocityRL"),
    ]
)

def _pem_key(key: rsa.RSAPrivateKey) -> bytes:
    return key.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.TraditionalOpenSSL,
        serialization.NoEncryption(),
    )

def load_or_create_ca() -> tuple[x509.Certificate, rsa.RSAPrivateKey]:
    if CA_CERT.exists() and CA_KEY.exists():
        cert = x509.load_pem_x509_certificate(CA_CERT.read_bytes())
        key = serialization.load_pem_private_key(CA_KEY.read_bytes(), password=None)

        if getattr(key, "key_size", 0) == 2048:
            print(f"[ok] loaded existing CA: {cert.subject.rfc4514_string()}")
            return cert, key
        print("[..] replacing 4096-bit CA with 2048-bit (RL OpenSSL)")

    now = datetime.datetime.now(datetime.timezone.utc)
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    builder = (
        x509.CertificateBuilder()
        .subject_name(CA_NAME)
        .issuer_name(CA_NAME)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=3650))
        .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=False,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=True,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), critical=False)
    )
    cert = builder.sign(key, hashes.SHA256())
    CA_CERT.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    CA_KEY.write_bytes(_pem_key(key))
    print(f"[ok] created VelocityRL root CA (2048) -> {CA_CERT.name}")
    return cert, key

def write_crl(ca_cert: x509.Certificate, ca_key: rsa.RSAPrivateKey) -> None:
    now = datetime.datetime.now(datetime.timezone.utc)
    ca_ski = x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key())
    crl = (
        x509.CertificateRevocationListBuilder()
        .issuer_name(ca_cert.subject)
        .last_update(now - datetime.timedelta(hours=1))
        .next_update(now + datetime.timedelta(days=365))
        .add_extension(x509.CRLNumber(1), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_subject_key_identifier(ca_ski), critical=False)
        .sign(ca_key, hashes.SHA256())
    )
    CRL_PATH.write_bytes(crl.public_bytes(serialization.Encoding.DER))
    print(f"[ok] wrote {CRL_PATH.name} (empty CRL)")

def mint_leaf(
    host: str,
    ca_cert: x509.Certificate,
    ca_key: rsa.RSAPrivateKey,
    extra_sans=None,
) -> tuple[bytes, bytes]:
    """Leaf shaped like the real config.psynet.gg cert (Google WR3), minus CT/AIA."""
    leaf_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    leaf_subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, host)])
    ca_ski = x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key())
    now = datetime.datetime.now(datetime.timezone.utc)
    names = [host]
    if extra_sans:
        names.extend(extra_sans)
    builder = (
        x509.CertificateBuilder()
        .subject_name(leaf_subject)
        .issuer_name(ca_cert.subject)
        .public_key(leaf_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=825))
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=True,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(leaf_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_subject_key_identifier(ca_ski), critical=False)
        .add_extension(x509.SubjectAlternativeName(sans_for(*names)), critical=False)
    )
    leaf = builder.sign(ca_key, hashes.SHA256())
    return leaf.public_bytes(serialization.Encoding.PEM), _pem_key(leaf_key)

def sans_for(*names: str) -> list[x509.GeneralName]:
    out: list[x509.GeneralName] = []
    seen: set[str] = set()
    for name in names:
        if name in seen:
            continue
        seen.add(name)
        try:
            out.append(x509.IPAddress(ipaddress.ip_address(name)))
        except ValueError:
            out.append(x509.DNSName(name))
    return out

def main() -> None:
    ca_cert, ca_key = load_or_create_ca()
    write_crl(ca_cert, ca_key)

    catch_names = HOSTS + ["localhost", "127.0.0.1", "::1"]
    pem, key = mint_leaf("config.psynet.gg", ca_cert, ca_key, extra_sans=catch_names)
    (HERE / "server.crt").write_bytes(pem)
    (HERE / "server.key").write_bytes(key)
    print(f"[ok] wrote server.crt SAN={', '.join(catch_names)}")

    for host in HOSTS:
        pem, key = mint_leaf(host, ca_cert, ca_key, extra_sans=["127.0.0.1", "::1", "localhost"])
        (HERE / f"leaf_{host}.crt").write_bytes(pem)
        (HERE / f"leaf_{host}.key").write_bytes(key)
        print(f"[ok] wrote leaf_{host}.crt")

    print("     signed by:", ca_cert.subject.rfc4514_string())
    write_openssl_hashes(ca_cert)

def _name_hash(der: bytes, algo: str) -> str:
    md = hashlib.new(algo, der).digest()
    h = md[0] | (md[1] << 8) | (md[2] << 16) | (md[3] << 24)
    return f"{h & 0xffffffff:08x}"

def write_openssl_hashes(ca_cert: x509.Certificate) -> None:
    """Local debug copies only — never install these system-wide.

    Planting a VelocityRL-only cert.pem / CAPATH hash into
    Common Files\\SSL or C:\\Windows previously broke EAC/EOS TLS.
    """
    pem = CA_CERT.read_bytes()
    der = ca_cert.subject.public_bytes()
    out = HERE / "openssl_trust"
    out.mkdir(exist_ok=True)
    (out / "cert.pem").write_bytes(pem)
    (out / "DO_NOT_INSTALL.txt").write_text(
        "Debug artifact only. Do NOT copy into Common Files\\SSL, "
        "C:\\Windows\\cert.pem, C:\\Windows\\certs, or any Rocket League "
        "game directory. Run fix_eac_trust.ps1 if those paths were polluted.\n",
        encoding="utf-8",
    )

    forbidden_parents = {
        Path(r"C:\Windows").resolve(),
        Path(r"C:\Program Files\Common Files\SSL").resolve(),
        Path(r"C:\Program Files (x86)\Common Files\SSL").resolve(),
    }
    resolved_out = out.resolve()
    for parent in forbidden_parents:
        try:
            resolved_out.relative_to(parent)
            raise RuntimeError(f"refusing to write openssl_trust under forbidden path {parent}")
        except ValueError:
            pass
    for algo in ("sha1", "md5"):
        name = _name_hash(der, algo) + ".0"
        dest = out / name
        if dest.resolve().parent != resolved_out:
            raise RuntimeError(f"refusing to write outside openssl_trust: {dest}")
        dest.write_bytes(pem)
        print(f"[ok] openssl hash {algo} -> openssl_trust/{name} (local only)")

if __name__ == "__main__":
    main()
