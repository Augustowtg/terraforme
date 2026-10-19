"""Testes da memória JSON do terraform-requisitos.

Rode da raiz da skill:
    python3 -m unittest discover -s scripts/tests -v
Os testes de PDF só rodam se houver 'uv' (ou reportlab instalado).
"""
import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
SKILL = SCRIPTS.parent
EXEMPLO_JSON = SKILL / "exemplo" / "requisitos.json"
EXEMPLO_MD = SKILL / "exemplo" / "exemplo-documento.md"
sys.path.insert(0, str(SCRIPTS))

import memoria  # noqa: E402
import validar_documento  # noqa: E402


def tem_gerador_pdf():
    if shutil.which("uv"):
        return True
    try:
        import reportlab  # noqa: F401
        import markdown_it  # noqa: F401
        return True
    except ImportError:
        return False


RF_MINIMO = {
    "titulo": "Montar pedido",
    "descricao": "O cliente pode adicionar produtos a um pedido.",
    "ator": "Cliente",
    "contexto": "Vendas",
    "criterios": ["Dado um pedido em rascunho, quando o cliente adiciona um produto, então o item aparece."],
}


class BaseMemoria(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="terraforme-teste-")
        self._env_antigo = os.environ.get("TERRAFORME_HOME")
        os.environ["TERRAFORME_HOME"] = self.tmp

    def tearDown(self):
        if self._env_antigo is None:
            os.environ.pop("TERRAFORME_HOME", None)
        else:
            os.environ["TERRAFORME_HOME"] = self._env_antigo
        shutil.rmtree(self.tmp, ignore_errors=True)

    def cli(self, *argv):
        """Roda a linha de comando e devolve (código, stdout, stderr)."""
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            codigo = memoria.main(list(argv))
        return codigo, out.getvalue().strip(), err.getvalue().strip()

    def ok(self, *argv):
        codigo, out, err = self.cli(*argv)
        self.assertEqual(codigo, 0, f"falhou: {argv}\n{out}\n{err}")
        return out

    def falha(self, *argv):
        codigo, out, err = self.cli(*argv)
        self.assertEqual(codigo, 1, f"deveria falhar: {argv}\n{out}")
        return err

    def json_salvo(self, projeto="loja"):
        return json.loads((Path(self.tmp) / projeto / "requisitos.json").read_text(encoding="utf-8"))

    def iniciar(self, projeto="loja"):
        self.ok("iniciar", projeto, "--titulo", "Loja Online", "--autor", "Equipe", "--data", "2026-10-16")


class TestIniciar(BaseMemoria):
    def test_cria_pasta_e_json_no_terraforme_home(self):
        self.iniciar()
        caminho = Path(self.tmp) / "loja" / "requisitos.json"
        self.assertTrue(caminho.is_file())
        dados = self.json_salvo()
        self.assertEqual(dados["esquema"], memoria.ESQUEMA)
        self.assertEqual(dados["documento"]["titulo"], "Loja Online")
        self.assertEqual(dados["versoes"], [{"versao": "1.0.0", "data": "2026-10-16", "autor": "Equipe",
                                             "mudancas": "Versão inicial"}])
        for secao in memoria.LISTAS:
            self.assertEqual(dados[secao], [])

    def test_nao_sobrescreve_projeto_existente(self):
        self.iniciar()
        self.assertIn("já existe", self.falha("iniciar", "loja", "--titulo", "Outra"))

    def test_recusa_nome_de_projeto_com_caminho(self):
        for nome in ("../fora", "a/b", ".oculto"):
            self.falha("iniciar", nome, "--titulo", "X")
        self.assertFalse(any(Path(self.tmp).parent.glob("fora")))

    def test_caminho_mostra_o_json(self):
        self.assertEqual(self.ok("caminho", "loja"), str(Path(self.tmp) / "loja" / "requisitos.json"))

    def test_comando_em_projeto_inexistente_falha(self):
        self.assertIn("não existe", self.falha("ler", "nada"))


class TestInserir(BaseMemoria):
    def setUp(self):
        super().setUp()
        self.iniciar()

    def test_gera_ids_sequenciais(self):
        self.assertEqual(self.ok("inserir", "loja", "rf", "--json", json.dumps(RF_MINIMO)), "RF-001")
        outro = {**RF_MINIMO, "titulo": "Confirmar pedido"}
        self.assertEqual(self.ok("inserir", "loja", "rf", "--json", json.dumps(outro)), "RF-002")
        self.assertEqual(self.json_salvo()["contadores"]["RF"], 2)

    def test_preenche_valores_padrao(self):
        self.ok("inserir", "loja", "rf", "--json", json.dumps(RF_MINIMO))
        rf = self.json_salvo()["requisitos_funcionais"][0]
        self.assertEqual(rf["regras"], [])
        self.assertEqual(rf["rastreabilidade"], [])
        self.assertIsNone(rf["removido_em"])

    def test_aceita_nome_longo_e_apelido_da_secao(self):
        self.ok("inserir", "loja", "requisitos_funcionais", "--json", json.dumps(RF_MINIMO))
        self.assertEqual(len(self.json_salvo()["requisitos_funcionais"]), 1)

    def test_recusa_campo_desconhecido(self):
        err = self.falha("inserir", "loja", "rf", "--json", json.dumps({**RF_MINIMO, "prioridade": "alta"}))
        self.assertIn("prioridade", err)

    def test_recusa_campo_obrigatorio_ausente(self):
        sem_criterio = {k: v for k, v in RF_MINIMO.items() if k != "criterios"}
        self.assertIn("criterios", self.falha("inserir", "loja", "rf", "--json", json.dumps(sem_criterio)))

    def test_recusa_tipo_de_codigo_que_nao_e_tipo(self):
        for tipo in ("Order.cancel", "src/order.ts", "cancel()"):
            termo = {"termo": "Pedido", "definicao": "d", "contexto": "Vendas", "tipo_codigo": tipo}
            self.falha("inserir", "loja", "glossario", "--json", json.dumps(termo))
        self.assertEqual(self.json_salvo()["glossario"], [])

    def test_recusa_termo_repetido_no_glossario(self):
        termo = {"termo": "Pedido", "definicao": "d", "contexto": "Vendas", "tipo_codigo": "Order"}
        self.ok("inserir", "loja", "glossario", "--json", json.dumps(termo))
        self.falha("inserir", "loja", "glossario", "--json", json.dumps({**termo, "termo": "pedido"}))

    def test_recusa_regra_com_id_mal_formado(self):
        self.falha("inserir", "loja", "rf", "--json", json.dumps({**RF_MINIMO, "regras": ["regra 1"]}))

    def test_recusa_id_ja_usado(self):
        self.ok("inserir", "loja", "rf", "--json", json.dumps(RF_MINIMO))
        self.falha("inserir", "loja", "rf", "--json", json.dumps({**RF_MINIMO, "id": "RF-001"}))

    def test_recusa_inserir_em_secao_de_objeto(self):
        self.falha("inserir", "loja", "visao_geral", "--json", '{"proposito": "x"}')

    def test_json_invalido(self):
        self.assertIn("--json", self.falha("inserir", "loja", "rf", "--json", "{nao é json"))


class TestEditarLerBuscar(BaseMemoria):
    def setUp(self):
        super().setUp()
        self.iniciar()
        self.ok("inserir", "loja", "rf", "--json", json.dumps(RF_MINIMO))

    def test_editar_um_campo_preserva_os_outros(self):
        antes = self.json_salvo()["requisitos_funcionais"][0]
        self.ok("editar", "loja", "rf", "RF-001", "--campo", "titulo", "--valor", "Montar carrinho")
        depois = self.json_salvo()["requisitos_funcionais"][0]
        self.assertEqual(depois["titulo"], "Montar carrinho")
        self.assertEqual({k: v for k, v in depois.items() if k != "titulo"},
                         {k: v for k, v in antes.items() if k != "titulo"})

    def test_editar_com_valor_json(self):
        self.ok("editar", "loja", "rf", "RF-001", "--campo", "regras", "--valor", '["RN-001"]')
        self.assertEqual(self.json_salvo()["requisitos_funcionais"][0]["regras"], ["RN-001"])

    def test_editar_mesclando_varios_campos(self):
        self.ok("editar", "loja", "rf", "RF-001", "--json", '{"ator": "Vendedor", "contexto": "Balcão"}')
        rf = self.json_salvo()["requisitos_funcionais"][0]
        self.assertEqual((rf["ator"], rf["contexto"]), ("Vendedor", "Balcão"))

    def test_anexar_em_lista(self):
        novo = "Dado um pedido cheio, quando o cliente adiciona, então o sistema recusa."
        self.ok("editar", "loja", "rf", "RF-001", "--campo", "criterios", "--valor", novo, "--anexar")
        self.assertEqual(self.json_salvo()["requisitos_funcionais"][0]["criterios"][-1], novo)

    def test_anexar_em_campo_que_nao_e_lista_falha(self):
        self.falha("editar", "loja", "rf", "RF-001", "--campo", "titulo", "--valor", "x", "--anexar")

    def test_editar_objeto_sem_id(self):
        self.ok("editar", "loja", "visao_geral", "--campo", "proposito", "--valor", "Vender online.")
        self.assertEqual(self.json_salvo()["visao_geral"]["proposito"], "Vender online.")

    def test_editar_recusa_campo_desconhecido_e_mudar_id(self):
        self.falha("editar", "loja", "rf", "RF-001", "--campo", "prioridade", "--valor", "alta")
        self.falha("editar", "loja", "rf", "RF-001", "--campo", "id", "--valor", "RF-009")
        self.falha("editar", "loja", "rf", "RF-404", "--campo", "titulo", "--valor", "x")

    def test_edicao_invalida_nao_altera_o_arquivo(self):
        antes = self.json_salvo()
        self.falha("editar", "loja", "rf", "RF-001", "--campo", "criterios", "--valor", "[]")
        self.assertEqual(self.json_salvo(), antes)

    def test_versoes_so_mudam_pelo_comando_versao(self):
        self.falha("editar", "loja", "versoes", "--campo", "versao", "--valor", "9.9.9")

    def test_ler_item_e_campo(self):
        item = json.loads(self.ok("ler", "loja", "rf", "RF-001"))
        self.assertEqual(item["titulo"], "Montar pedido")
        self.assertEqual(self.ok("ler", "loja", "rf", "RF-001", "--campo", "ator"), "Cliente")
        self.assertEqual(len(json.loads(self.ok("ler", "loja", "rf"))), 1)

    def test_buscar_ignora_acentos_e_maiusculas(self):
        achados = json.loads(self.ok("buscar", "loja", "ENTAO O ITEM", "--json"))
        self.assertEqual([(a["secao"], a["id"], a["campo"]) for a in achados],
                         [("requisitos_funcionais", "RF-001", "criterios[0]")])

    def test_buscar_filtra_por_secao(self):
        self.assertEqual(json.loads(self.ok("buscar", "loja", "pedido", "--secao", "rn", "--json")), [])
        self.assertEqual(self.ok("buscar", "loja", "inexistente"), "Nada encontrado.")


class TestRemoverEVersao(BaseMemoria):
    def setUp(self):
        super().setUp()
        self.iniciar()
        for titulo in ("A", "B"):
            self.ok("inserir", "loja", "rf", "--json", json.dumps({**RF_MINIMO, "titulo": titulo}))

    def test_remover_rf_marca_e_nao_reaproveita_id(self):
        self.ok("versao", "loja", "--tipo", "maior", "--mudancas", "Remoção do RF-002", "--data", "2026-10-17")
        self.ok("remover", "loja", "rf", "RF-002")
        dados = self.json_salvo()
        self.assertEqual(dados["requisitos_funcionais"][1]["removido_em"], "2.0.0")
        self.assertEqual(self.ok("inserir", "loja", "rf", "--json", json.dumps(RF_MINIMO)), "RF-003")

    def test_remover_item_comum_apaga_mas_id_nao_volta(self):
        termo = {"termo": "Pedido", "definicao": "d", "contexto": "Vendas", "tipo_codigo": "Order"}
        self.assertEqual(self.ok("inserir", "loja", "glossario", "--json", json.dumps(termo)), "GLO-001")
        self.ok("remover", "loja", "glossario", "GLO-001")
        self.assertEqual(self.json_salvo()["glossario"], [])
        self.assertEqual(self.ok("inserir", "loja", "glossario", "--json", json.dumps(termo)), "GLO-002")

    def test_tipos_de_versao(self):
        self.assertEqual(self.ok("versao", "loja", "--tipo", "correcao", "--mudancas", "Redação"), "1.0.1")
        self.assertEqual(self.ok("versao", "loja", "--tipo", "menor", "--mudancas", "RF novo"), "1.1.0")
        self.assertEqual(self.ok("versao", "loja", "--tipo", "maior", "--mudancas", "RF removido"), "2.0.0")
        versoes = self.json_salvo()["versoes"]
        self.assertEqual([v["versao"] for v in versoes], ["1.0.0", "1.0.1", "1.1.0", "2.0.0"])
        self.assertEqual(versoes[-1]["autor"], "Equipe")

    def test_versao_recusa_data_invalida(self):
        self.falha("versao", "loja", "--tipo", "menor", "--mudancas", "x", "--data", "16/10/2026")


class TestGerar(BaseMemoria):
    def importar_exemplo(self):
        self.ok("importar", "loja-online", "--arquivo", str(EXEMPLO_JSON))

    def test_exemplo_json_gera_exatamente_o_exemplo_md(self):
        """Mantém exemplo/requisitos.json e exemplo/exemplo-documento.md em sincronia."""
        self.importar_exemplo()
        saida = self.ok("gerar", "loja-online")
        md = Path(self.tmp) / "loja-online" / "requisitos.md"
        self.assertIn(str(md), saida)
        self.assertEqual(md.read_text(encoding="utf-8"), EXEMPLO_MD.read_text(encoding="utf-8"))

    def test_md_gerado_passa_no_validador(self):
        self.importar_exemplo()
        self.ok("editar", "loja-online", "rf", "RF-003", "--campo", "titulo", "--valor", "Cancelar antes do envio")
        self.ok("gerar", "loja-online")
        erros, avisos, rf, rn, versao = validar_documento.validar(Path(self.tmp) / "loja-online" / "requisitos.md")
        self.assertEqual((erros, avisos), ([], []))
        self.assertEqual((rf, rn, versao), (5, 6, "1.1.0"))

    def test_edicao_aparece_no_md(self):
        self.importar_exemplo()
        self.ok("editar", "loja-online", "rn", "RN-002", "--campo", "regra",
                "--valor", "Um pedido pode ter no máximo 80 itens.")
        self.ok("versao", "loja-online", "--tipo", "maior", "--mudancas", "RN-002 passa a 80 itens")
        self.ok("gerar", "loja-online")
        md = (Path(self.tmp) / "loja-online" / "requisitos.md").read_text(encoding="utf-8")
        self.assertIn("- **Regra:** Um pedido pode ter no máximo 80 itens.", md)
        self.assertIn("- **Versão do documento:** 2.0.0", md)

    def test_item_removido_continua_no_md_com_marca(self):
        self.importar_exemplo()
        self.ok("versao", "loja-online", "--tipo", "maior", "--mudancas", "Remoção do RF-004")
        self.ok("remover", "loja-online", "rf", "RF-004")
        self.ok("gerar", "loja-online")
        md_path = Path(self.tmp) / "loja-online" / "requisitos.md"
        self.assertIn("### RF-004 — Registrar resultado da cobrança (removido na versão 2.0.0)",
                      md_path.read_text(encoding="utf-8"))
        self.assertEqual(validar_documento.validar(md_path)[0], [])

    def test_gerar_recusa_rastreabilidade_fora_do_glossario(self):
        self.importar_exemplo()
        self.ok("editar", "loja-online", "rf", "RF-001", "--campo", "rastreabilidade",
                "--valor", "Carrinho", "--anexar")
        self.assertIn("Carrinho", self.falha("gerar", "loja-online"))

    def test_gerar_recusa_termo_sem_tipo_na_rastreabilidade(self):
        self.importar_exemplo()
        self.ok("editar", "loja-online", "rn", "RN-003", "--campo", "rastreabilidade", "--valor", '["Confirmar pedido"]')
        self.assertIn("não tem tipo", self.falha("gerar", "loja-online"))

    def test_gerar_recusa_regra_inexistente(self):
        self.importar_exemplo()
        self.ok("editar", "loja-online", "rf", "RF-001", "--campo", "regras", "--valor", '["RN-099"]')
        self.assertIn("RN-099", self.falha("gerar", "loja-online"))

    def test_gerar_md_em_outro_caminho(self):
        self.importar_exemplo()
        destino = Path(self.tmp) / "repo" / "docs" / "requisitos.md"
        self.ok("gerar", "loja-online", "--md", str(destino))
        self.assertTrue(destino.is_file())

    def test_projeto_novo_sem_rf_nao_gera(self):
        self.iniciar()
        self.assertIn("nenhum requisito funcional", self.falha("gerar", "loja"))

    def test_importar_nao_sobrescreve_sem_flag(self):
        self.importar_exemplo()
        self.falha("importar", "loja-online", "--arquivo", str(EXEMPLO_JSON))
        self.ok("importar", "loja-online", "--arquivo", str(EXEMPLO_JSON), "--substituir")

    def test_importar_preserva_contadores(self):
        self.importar_exemplo()
        self.assertEqual(self.ok("inserir", "loja-online", "rf", "--json", json.dumps(RF_MINIMO)), "RF-006")

    @unittest.skipUnless(tem_gerador_pdf(), "sem uv nem reportlab para gerar PDF")
    def test_gera_pdf_padrao(self):
        self.importar_exemplo()
        self.ok("gerar", "loja-online", "--pdf")
        pdf = Path(self.tmp) / "loja-online" / "requisitos.pdf"
        self.assertTrue(pdf.is_file(), "requisitos.pdf")
        self.assertEqual(pdf.read_bytes()[:5], b"%PDF-")


if __name__ == "__main__":
    unittest.main()
