import { describe, expect, it } from 'vitest';
import { sealedBoxEncrypt } from './crypto';

// ── sealedBoxEncrypt ──────────────────────────────────────────────────────────

/** Generate a fresh P-256 key pair and return the public key as standard base64. */
async function generateTestPublicKeyBase64(): Promise<string> {
	const keyPair = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
	const raw = new Uint8Array(await crypto.subtle.exportKey('raw', keyPair.publicKey));
	let binary = '';
	for (const byte of raw) binary += String.fromCharCode(byte);
	return btoa(binary);
}

describe('sealedBoxEncrypt', () => {
	it('returns a non-empty base64 string', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result = await sealedBoxEncrypt('hello', pubKey);
		expect(typeof result).toBe('string');
		expect(result.length).toBeGreaterThan(0);
		// Standard base64 character set (may include padding).
		expect(result).toMatch(/^[A-Za-z0-9+/]+=*$/);
	});

	it('produces a sealed box of the correct minimum binary length', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const plaintext = 'test message';
		const result = await sealedBoxEncrypt(plaintext, pubKey);
		// Decode and verify: 65 (ephemeral pubkey) + 12 (nonce) + plaintext.length + 16 (GCM tag).
		const decoded = atob(result);
		const minLen = 65 + 12 + new TextEncoder().encode(plaintext).length + 16;
		expect(decoded.length).toBe(minLen);
	});

	it('produces different ciphertext on each call (fresh ephemeral keypair)', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result1 = await sealedBoxEncrypt('same plaintext', pubKey);
		const result2 = await sealedBoxEncrypt('same plaintext', pubKey);
		expect(result1).not.toBe(result2);
	});

	it('encodes the ephemeral uncompressed P-256 public key as the first 65 bytes (starts with 0x04)', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result = await sealedBoxEncrypt('hello', pubKey);
		const decoded = atob(result);
		// Uncompressed P-256 points start with the 0x04 prefix byte.
		expect(decoded.charCodeAt(0)).toBe(0x04);
	});

	it('throws for invalid base64 input', async () => {
		await expect(sealedBoxEncrypt('hello', '!!not-valid-base64!!')).rejects.toThrow();
	});

	it('throws when the decoded public key is not a valid P-256 point', async () => {
		// 65 bytes of zeros is not a valid uncompressed P-256 point.
		const invalidKey = btoa(String.fromCharCode(...new Array(65).fill(0)));
		await expect(sealedBoxEncrypt('hello', invalidKey)).rejects.toThrow();
	});

	it('works with an empty plaintext', async () => {
		const pubKey = await generateTestPublicKeyBase64();
		const result = await sealedBoxEncrypt('', pubKey);
		// 65 + 12 + 0 + 16 = 93 bytes.
		const decoded = atob(result);
		expect(decoded.length).toBe(93);
	});
});
