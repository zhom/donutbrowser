FROM node:22-alpine

WORKDIR /app

COPY package.json .
COPY dist/ dist/
COPY node_modules/ node_modules/

ENV NODE_ENV=production
EXPOSE 12342

CMD ["node", "dist/main"]
