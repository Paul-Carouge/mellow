#!/bin/bash
# Makes the self-signed code-signing identity Mellow.app is signed with (ADR-0055), in a new
# directory: `identity.p12` and its base64 form for the release environment's secret, its
# password, and the certificate alone. The maintainer runs it once; CI runs it on pull requests to
# try the signing path with a throwaway identity.
#
#   make-signing-certificate.sh <new directory>
set -euo pipefail

output_directory="${1:?usage: make-signing-certificate.sh <new directory>}"
# The name codesign shows; the designated requirement pins the certificate itself, not this name.
common_name="Mellow Desktop"
# A new certificate resets every user's camera, microphone and screen permissions, so it is made
# to outlive the project rather than to expire and force a rotation.
validity_days=36500

if [[ -e "${output_directory}" ]]; then
  echo "${output_directory} already exists; give a new directory" >&2
  exit 1
fi
umask 077
mkdir -p "${output_directory}"
cd "${output_directory}"

cat > certificate.cnf <<EOF
[req]
distinguished_name = name
x509_extensions = code_signing
prompt = no
[name]
CN = ${common_name}
[code_signing]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF

openssl req -x509 -newkey rsa:3072 -sha256 -nodes -days "${validity_days}" \
  -config certificate.cnf -keyout private-key.pem -out certificate.pem 2> /dev/null
password="$(openssl rand -hex 24)"
printf '%s' "${password}" > password.txt
# SHA-1 and 3DES: the PKCS#12 algorithms every macOS `security import` reads.
openssl pkcs12 -export -inkey private-key.pem -in certificate.pem -name "${common_name}" \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
  -passout file:password.txt -out identity.p12
base64 < identity.p12 | tr -d '\n' > identity.p12.base64
rm -f private-key.pem certificate.cnf

echo "made ${output_directory}: identity.p12(.base64), password.txt, certificate.pem"
openssl x509 -in certificate.pem -noout -subject -enddate -fingerprint -sha256
