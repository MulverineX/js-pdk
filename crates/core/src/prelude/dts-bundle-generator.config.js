// @ts-check

/** @type import('dts-bundle-generator/config-schema').BundlerConfig */
const config = {
  compilationOptions: {
    preferredConfigPath: './tsconfig.json',
  },

  entries: [
    {
      filePath: './src/index.ts',
      outFile: './dist/index.d.ts',
      noCheck: true,
      output: {
        inlineDeclareGlobals: true,
      },
    },
  ],
};

module.exports = config;