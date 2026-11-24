# Assinador para WhatsApp

Aplicativo Rust para Windows 10 e 11 (x64). Em uma conta de WhatsApp usada por várias pessoas, o nome do operador ativo aparece em negrito na primeira linha da mensagem.

> **Ana:**  
> Olá, como posso ajudar?

1. Abra `Assinador-WhatsApp-x64.msi`, aguarde a confirmação e depois abra **Assinador para WhatsApp** no menu Iniciar.
2. Digite um nome e clique em **Adicionar**. Para ativar outro perfil, selecione-o e clique em **Usar este nome**. O perfil em uso aparece com **(atual)** na lista.
3. No WhatsApp instalado pela Microsoft Store, escreva no campo da conversa e envie normalmente. O Assinador observa o envio por **Enter** ou pelo botão **Enviar** quando o campo da conversa está em foco.
4. Feche a janela do Assinador para deixá-lo aberto ao lado do relógio, na seta **^**. Pressione **Alt+W** para reabrir a janela de nomes. Use o botão direito no ícone para trocar de operador, pausar ou sair.

O instalador cria um atalho no menu Iniciar e inicia o Assinador em segundo plano quando o usuário entra no Windows. Os nomes e o operador selecionado ficam no computador. A assinatura cobre o campo de texto das conversas em português e em inglês. A integração depende dos controles de acessibilidade que a versão instalada do WhatsApp expõe.

## Compilar

Instale o Rust com o toolchain MSVC x64, as ferramentas de compilação C++ do Visual Studio e o WiX 6. Na pasta do projeto, execute:

```powershell
cargo test --locked
wix extension add -g WixToolset.UI.wixext/6.0.2
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\installer\build.ps1
```

O instalador é gerado em `target\installer\Assinador-WhatsApp-x64.msi`.
