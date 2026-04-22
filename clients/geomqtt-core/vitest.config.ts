import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    // Tests import from ../src/*.js (TypeScript ESM convention).
    // vitest resolves these to the .ts source automatically.
  },
});
