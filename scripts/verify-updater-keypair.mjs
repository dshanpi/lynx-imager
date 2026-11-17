#!/usr/bin/env node

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

function requireValue(condition, message) {
  if (!condition) throw new Error(message);
}

function packetKeyId(packet, label) {
  requireValue(packet.length >= 10, `${label} packet is too short.`);
  return Buffer.from(packet.subarray(2, 10)).reverse().toString('hex').toUpperCase();
}

export function signatureKeyId(encodedSignature) {
  const minisignText = Buffer.from(encodedSignature.trim(), 'base64').toString('utf8');
  const lines = minisignText.trim().split(/\r?\n/);
  requireValue(lines.length >= 2, 'Updater signature format is invalid.');
  return packetKeyId(Buffer.from(lines[1], 'base64'), 'Updater signature');
}

export function configuredPublicKeyId(encodedPublicKey) {
  const minisignText = Buffer.from(encodedPublicKey.trim(), 'base64').toString('utf8');
  const publicKey = minisignText.split(/\r?\n/).find((line) => line.startsWith('RW'));
  requireValue(publicKey, 'Configured updater public key is invalid.');
  return packetKeyId(Buffer.from(publicKey, 'base64'), 'Updater public key');
}

export function verifyUpdaterKeyPair(signaturePath, configPath = 'src-tauri/tauri.conf.json') {
  const config = JSON.parse(readFileSync(configPath, 'utf8'));
  const signatureId = signatureKeyId(readFileSync(signaturePath, 'utf8'));
  const publicId = configuredPublicKeyId(config.plugins?.updater?.pubkey || '');
  requireValue(
    signatureId === publicId,
    `Updater signing key ${signatureId} does not match configured public key ${publicId}.`
  );
  console.log(`Updater signing key matches configured public key: ${publicId}.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [signaturePath, configPath] = process.argv.slice(2);
  requireValue(signaturePath, 'Usage: verify-updater-keypair.mjs <probe.sig> [tauri.conf.json]');
  verifyUpdaterKeyPair(signaturePath, configPath);
}
