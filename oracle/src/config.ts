import { Keypair } from '@stellar/stellar-sdk';
import { decodeSecretKey } from './keys/secret-key';

export interface OracleConfig {
  secretKey: string;
  rpcUrl: string;
  factoryContractId: string;
  logLevel: string;
  pollIntervalMs: number;
}

function isValidSecretKey(secret: string): boolean {
  const trimmed = secret.trim();

  // Accept Stellar S... secrets directly.
  if (trimmed.startsWith('S')) {
    try {
      Keypair.fromSecret(trimmed);
      return true;
    } catch {
      return false;
    }
  }

  // Reuse existing decoder support (hex/base64) and enforce 32-byte seed.
  try {
    const decoded = decodeSecretKey(trimmed);
    return decoded.length === 32;
  } catch {
    return false;
  }
}

export function loadAndValidateConfig(): OracleConfig {
  const errors: string[] = [];

  const secretKey = process.env.ORACLE_SECRET_KEY;
  if (!secretKey) {
    errors.push('ORACLE_SECRET_KEY is required');
  } else if (!isValidSecretKey(secretKey)) {
    errors.push('ORACLE_SECRET_KEY is not a valid Ed25519 secret key');
  }

  const rpcUrl = process.env.STELLAR_RPC_URL;
  if (!rpcUrl) {
    errors.push('STELLAR_RPC_URL is required');
  }

  const factoryContractId = process.env.FACTORY_CONTRACT_ID;
  if (!factoryContractId) {
    errors.push('FACTORY_CONTRACT_ID is required');
  }

  const rawPollInterval = process.env.POLL_INTERVAL_MS ?? process.env.ORACLE_POLL_INTERVAL_MS ?? '5000';
  const pollIntervalMs = Number(rawPollInterval);
  if (!Number.isFinite(pollIntervalMs) || pollIntervalMs <= 0) {
    errors.push('POLL_INTERVAL_MS must be a positive number');
  }

  if (errors.length > 0) {
    console.error('Configuration errors:');
    for (const error of errors) {
      console.error(` - ${error}`);
    }
    process.exit(1);
  }

  return {
    secretKey: secretKey!,
    rpcUrl: rpcUrl!,
    factoryContractId: factoryContractId!,
    logLevel: process.env.LOG_LEVEL ?? 'info',
    pollIntervalMs,
  };
}
