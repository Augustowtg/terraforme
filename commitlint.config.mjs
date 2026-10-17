// Conventional Commits: https://www.conventionalcommits.org
// Formato: <type>(<scope>): <subject>   ex.: feat(cli): add init command

// Mensagens de commit devem ser em inglês. Como não há detecção de idioma
// confiável, bloqueamos caracteres fora do ASCII (á, ç, ã, õ...) e algumas
// palavras comuns em português.
const PT_WORDS =
  /\b(adiciona|adicionado|adicionar|corrige|corrigido|corrigir|cria|criado|criar|ajusta|ajuste|atualiza|atualizado|atualizar|removido|melhoria|melhora|inclui|incluir|altera|alterar|configura|configurar|para|nao|uma|pelo|pela|quando|tela|arquivo)\b/i

const englishOnly = (parsed) => {
  const text = [parsed.header, parsed.body, parsed.footer].filter(Boolean).join('\n')
  if (/[^\x00-\x7F]/.test(text)) {
    return [false, 'commit message must be in English (non-ASCII characters found)']
  }
  const match = text.match(PT_WORDS)
  if (match) {
    return [false, `commit message must be in English (found Portuguese word "${match[0]}")`]
  }
  return [true]
}

export default {
  extends: ['@commitlint/config-conventional'],
  plugins: [{ rules: { 'english-only': englishOnly } }],
  rules: {
    'english-only': [2, 'always'],
    'scope-enum': [2, 'always', ['core', 'cli', 'deps', 'ci', 'docs', 'config', 'release']],
    'subject-case': [2, 'always', 'lower-case'],
    'header-max-length': [2, 'always', 100],
  },
}
