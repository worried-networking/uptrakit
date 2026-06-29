function bytesToBase64(bytes: Uint8Array): string {
	let binary = '';
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary);
}

function base64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
	const binary = atob(b64);
	const bytes = new Uint8Array(new ArrayBuffer(binary.length));
	for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

/**
 * ECIES sealed-box encrypt using the Web Crypto API (P-256 ECDH + AES-256-GCM).
 *
 * Matches the Rust `sealed_box_encrypt_base64` algorithm exactly:
 * - Ephemeral P-256 keypair per message (forward secrecy).
 * - Shared secret = ECDH X-coordinate (32 bytes).
 * - AES-256 key = SHA-256(shared secret).
 * - AAD = ephemeral public key bytes (65 bytes, uncompressed).
 * - Sealed-box format: [ephemeral pubkey (65)] [nonce (12)] [ciphertext + GCM tag (N+16)].
 * - Returns standard (non-URL-safe) base64.
 *
 * @param plaintext - The UTF-8 string to encrypt.
 * @param recipientPublicKeyBase64 - Standard base64-encoded uncompressed P-256 public key (65 bytes).
 */
export async function sealedBoxEncrypt(plaintext: string, recipientPublicKeyBase64: string): Promise<string> {
	const recipientPubKeyBytes = base64ToBytes(recipientPublicKeyBase64);

	// Import recipient's static P-256 public key for ECDH.
	const recipientPublicKey = await crypto.subtle.importKey(
		'raw',
		recipientPubKeyBytes,
		{ name: 'ECDH', namedCurve: 'P-256' },
		false,
		[]
	);

	// Generate ephemeral P-256 keypair (fresh per message).
	const ephemeralKeyPair = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, false, [
		'deriveBits'
	]);

	// Export ephemeral public key (uncompressed, 65 bytes: 0x04 || x || y).
	const ephemeralPubKeyRaw = new Uint8Array(await crypto.subtle.exportKey('raw', ephemeralKeyPair.publicKey));

	// ECDH: derive 32-byte shared secret (X-coordinate of the shared point).
	const sharedSecretBits = await crypto.subtle.deriveBits(
		{ name: 'ECDH', public: recipientPublicKey },
		ephemeralKeyPair.privateKey,
		256
	);

	// Key derivation: AES-256 key = SHA-256(shared secret).
	const aesKeyMaterial = await crypto.subtle.digest('SHA-256', sharedSecretBits);
	const aesKey = await crypto.subtle.importKey('raw', aesKeyMaterial, 'AES-GCM', false, ['encrypt']);

	// AES-256-GCM encrypt with random nonce and ephemeral public key as AAD.
	const nonce = crypto.getRandomValues(new Uint8Array(12));
	const ciphertextWithTag = new Uint8Array(
		await crypto.subtle.encrypt(
			{ name: 'AES-GCM', iv: nonce, additionalData: ephemeralPubKeyRaw },
			aesKey,
			new TextEncoder().encode(plaintext)
		)
	);

	// Assemble: ephemeral_pub_key (65) || nonce (12) || ciphertext+tag.
	const sealed = new Uint8Array(65 + 12 + ciphertextWithTag.length);
	sealed.set(ephemeralPubKeyRaw, 0);
	sealed.set(nonce, 65);
	sealed.set(ciphertextWithTag, 77);

	return bytesToBase64(sealed);
}
