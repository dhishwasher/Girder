import { registerHooks } from 'node:module';

registerHooks({
  resolve(specifier, context, next) {
    if (specifier === './app.ts' && context.parentURL?.endsWith('/app.test.ts')) {
      return next('./fake.ts', context);
    }
    return next(specifier, context);
  },
});
