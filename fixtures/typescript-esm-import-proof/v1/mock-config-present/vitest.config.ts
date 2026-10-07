export default {
  test: { setupFiles: ['./setup.ts'] },
  resolve: { alias: { './app.ts': './fake.ts' } },
};
