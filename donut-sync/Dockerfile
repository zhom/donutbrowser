FROM node:22-alpine AS builder

WORKDIR /build
COPY donut-sync/package.json donut-sync/tsconfig.json donut-sync/tsconfig.build.json ./
COPY donut-sync/src/ src/
RUN npm install
RUN npm run build
RUN npm prune --omit=dev

FROM node:22-alpine

WORKDIR /app
COPY --from=builder /build/package.json .
COPY --from=builder /build/dist/ dist/
COPY --from=builder /build/node_modules/ node_modules/

ENV NODE_ENV=production
EXPOSE 12342

USER node
CMD ["node", "dist/main"]
