# Dockerfile
FROM node:20-slim

# Instala ferramentas essenciais no terminal para que o executor shell do agente funcione corretamente
RUN apt-get update && apt-get install -y \
    git \
    curl \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copia manifestos de dependências primeiro para cacheamento de camadas
COPY package*.json ./

# Instala apenas dependências de produção para manter a imagem leve
RUN npm ci --only=production

# Copia os diretórios de código fonte e entrypoint
COPY src/ ./src/
COPY index.js ./
COPY .env.example ./

# Cria os diretórios necessários para a persistência de dados
RUN mkdir -p tasks/sessions workspace

# Configurações de ambiente padrão
ENV NODE_ENV=production
ENV REST_PORT=3120

# Expõe a porta default para a API REST
EXPOSE 3120

# Comando para iniciar o orquestrador
CMD ["node", "index.js"]
