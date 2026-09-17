#!/usr/bin/env python3
"""Generate VelocityRL CA + PsyNet leaf certs for the native Rust proxy.

Outputs (next to this script):
  velocityrl_ca.crt
  leaf_config.psynet.gg.{crt,key}
  leaf_ws.rlpp.psynet.gg.{crt,key}

Leaf private keys are required at compile time (include_bytes! in proxy.rs).
velocityrl_ca.key is only needed to mint new leaves later — keep it local.
"""

from __future__ import annotations

import datetime
import hashlib
import sys
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

HERE = Path(__file__).resolve().parent
CA_CERT = HERE / "velocityrl_ca.crt"
CA_KEY = HERE / "velocityrl_ca.key"

LEAVES = [
    "config.psynet.gg",
    "ws.rlpp.psynet.gg",
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


def _pem_cert(cert: x509.Certificate) -> bytes:
    return cert.public_bytes(serialization.Encoding.PEM)


def sha1_thumbprint(cert: x509.Certificate) -> str:
    return hashlib.sha1(cert.public_bytes(serialization.Encoding.DER)).hexdigest().upper()


def load_or_create_ca() -> tuple[x509.Certificate, rsa.RSAPrivateKey]:
    if CA_CERT.exists() and CA_KEY.exists():
        cert = x509.load_pem_x509_certificate(CA_CERT.read_bytes())
        key = serialization.load_pem_private_key(CA_KEY.read_bytes(), password=None)
        print(f"[ok] loaded existing CA thumbprint={sha1_thumbprint(cert)}")
        return cert, key

    now = datetime.datetime.now(datetime.timezone.utc)
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    cert = (
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
        .sign(key, hashes.SHA256())
    )
    CA_CERT.write_bytes(_pem_cert(cert))
    CA_KEY.write_bytes(_pem_key(key))
    print(f"[ok] created CA thumbprint={sha1_thumbprint(cert)}")
    return cert, key


def issue_leaf(ca_cert: x509.Certificate, ca_key: rsa.RSAPrivateKey, host: str) -> None:
    now = datetime.datetime.now(datetime.timezone.utc)
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, host)])
    cert = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(ca_cert.subject)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=825))
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.SubjectAlternativeName([x509.DNSName(host)]),
            critical=False,
        )
        .add_extension(
            x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]),
            critical=False,
        )
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
        .sign(ca_key, hashes.SHA256())
    )
    stem = f"leaf_{host}"
    (HERE / f"{stem}.crt").write_bytes(_pem_cert(cert))
    (HERE / f"{stem}.key").write_bytes(_pem_key(key))
    print(f"[ok] wrote {stem}.crt/.key")


def main() -> int:
    ca_cert, ca_key = load_or_create_ca()
    for host in LEAVES:
        issue_leaf(ca_cert, ca_key, host)
    thumb = sha1_thumbprint(ca_cert)
    print(f"[ok] BUNDLED_CA_THUMBPRINT={thumb}")
    print(
        "[note] Update src-tauri/src/psynet.rs BUNDLED_CA_THUMBPRINT if the CA was recreated."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
