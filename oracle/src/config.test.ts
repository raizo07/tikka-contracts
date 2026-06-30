import { Keypair } from '@stellar/stellar-sdk';
import { loadAndValidateConfig } from './config';

describe('loadAndValidateConfig', () => {
  const originalEnv = process.env;
  let exitSpy: jest.SpyInstance;
  let errorSpy: jest.SpyInstance;

  beforeEach(() => {
    process.env = { ...originalEnv };
    delete process.env.ORACLE_SECRET_KEY;
    delete process.env.STELLAR_RPC_URL;
    delete process.env.FACTORY_CONTRACT_ID;
    delete process.env.POLL_INTERVAL_MS;
    delete process.env.ORACLE_POLL_INTERVAL_MS;
    delete process.env.LOG_LEVEL;

    exitSpy = jest
      .spyOn(process, 'exit')
      .mockImplementation(((code?: number) => {
        throw new Error(`process.exit:${code ?? 0}`);
      }) as never);
    errorSpy = jest.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    process.env = originalEnv;
    exitSpy.mockRestore();
    errorSpy.mockRestore();
  });

  it('exits with code 1 when required env vars are missing', () => {
    expect(() => loadAndValidateConfig()).toThrow('process.exit:1');
    expect(errorSpy).toHaveBeenCalledWith('Configuration errors:');
    expect(errorSpy).toHaveBeenCalledWith(' - ORACLE_SECRET_KEY is required');
    expect(errorSpy).toHaveBeenCalledWith(' - STELLAR_RPC_URL is required');
    expect(errorSpy).toHaveBeenCalledWith(' - FACTORY_CONTRACT_ID is required');
  });

  it('exits with code 1 when secret key format is invalid', () => {
    process.env.ORACLE_SECRET_KEY = 'not-a-secret';
    process.env.STELLAR_RPC_URL = 'https://soroban-testnet.stellar.org';
    process.env.FACTORY_CONTRACT_ID = 'CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M';

    expect(() => loadAndValidateConfig()).toThrow('process.exit:1');
    expect(errorSpy).toHaveBeenCalledWith(
      ' - ORACLE_SECRET_KEY is not a valid Ed25519 secret key',
    );
  });

  it('returns validated config when env is valid', () => {
    process.env.ORACLE_SECRET_KEY = Keypair.random().secret();
    process.env.STELLAR_RPC_URL = 'https://soroban-testnet.stellar.org';
    process.env.FACTORY_CONTRACT_ID = 'CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M';
    process.env.POLL_INTERVAL_MS = '7000';
    process.env.LOG_LEVEL = 'debug';

    const config = loadAndValidateConfig();

    expect(config.rpcUrl).toBe(process.env.STELLAR_RPC_URL);
    expect(config.factoryContractId).toBe(process.env.FACTORY_CONTRACT_ID);
    expect(config.logLevel).toBe('debug');
    expect(config.pollIntervalMs).toBe(7000);
  });
});