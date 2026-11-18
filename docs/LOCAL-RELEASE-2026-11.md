# Atualização local do 1CE Desktop — novembro de 2026

Este registro descreve o build instalado e verificado no Windows. A [PR #3](https://github.com/tofdidthat/1ce-desktop/pull/3) continua aberta e antecede estes ajustes. O estado do app local não representa o conteúdo de `main`.

- **Beat Queue** aceita projetos FL Studio e arquivos MP3/WAV por arraste. Find Key roda em lote, continua se um item falhar e pula títulos com tonalidade explícita. Estimativas exigem revisão antes de entrar no nome. Rename preserva os bytes, a extensão e a pasta do arquivo, sem sobrescrever destinos. A análise de MP3 usa até os primeiros 60 segundos.
- **Export** entrega stems e WAV master em `Projects/<projeto>/stems`. O destino de MP3 continua separado. A operação preserva arquivos já presentes e recusa colisões. A coluna de configuração para o destino separado de stems foi removida.
- **Migração local** moveu 171 WAV de 15 lotes, com hashes conferidos. Dois lotes e um ZIP ficaram no destino antigo por falta de correspondência confirmada em Projects.
- **Today Tasks** mostra a aba de 52 × 12 em repouso e revela o rótulo em 100 × 24 quando o cursor se aproxima. Abre por clique ou teclado e fecha ao clicar fora, inclusive com o campo focado. Arraste e persistência foram verificados.
- **Organize** mantém a instrução de arraste no corpo da fila vazia e remove textos repetidos do rodapé e selos decorativos.

O build integrado passou em 92 testes Rust (7 ignorados) e 11 JS. O instalador final foi aplicado e a interface principal foi conferida no binário instalado. Dez ciclos de Today Tasks não mostraram reabertura nas amostras. Não houve validação quadro a quadro nem exportação real do FL Studio nesta rodada.
