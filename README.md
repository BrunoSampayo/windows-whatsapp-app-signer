# Assinador para WhatsApp

Aplicativo Rust para Windows 10 e 11 (x64). Em uma conta de WhatsApp usada por várias pessoas, o nome do operador ativo aparece em negrito na primeira linha da mensagem.

> **Ana:**  
> Olá, como posso ajudar?

1. Abra `Assinador-WhatsApp-x64.msi`, aguarde a confirmação e depois abra **Assinador para WhatsApp** no menu Iniciar.
2. Digite um nome e clique em **Adicionar**. Para ativar outro perfil, selecione-o e clique em **Usar este nome**. O perfil em uso aparece com **(atual)** na lista.
3. Em **Versão do WhatsApp**, selecione **Aplicativo do Windows** para usar a versão da Microsoft Store ou **Navegador (WhatsApp Web)** para usar o site `web.whatsapp.com` no Edge ou no Chrome. A escolha é salva e pode ser alterada na mesma janela.
4. Escreva no campo da conversa e envie normalmente por **Enter** ou pelo botão **Enviar**, com o campo da conversa em foco.
5. Feche a janela do Assinador para deixá-lo aberto ao lado do relógio, na seta **^**. Pressione **Alt+W** para reabrir a janela de nomes. Use o botão direito no ícone para trocar de operador, pausar ou sair.

O instalador cria um atalho no menu Iniciar e inicia o Assinador em segundo plano quando o usuário entra no Windows. Os nomes, o operador e a versão selecionada ficam no computador. A assinatura cobre o campo de texto das conversas em português e em inglês. A integração usa os controles de acessibilidade do aplicativo do Windows ou da aba do WhatsApp Web.

## Compilar

Instale o Rust com o toolchain MSVC x64, as ferramentas de compilação C++ do Visual Studio e o WiX 6. Na pasta do projeto, execute:

```powershell
cargo test --locked
wix extension add -g WixToolset.UI.wixext/6.0.2
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\installer\build.ps1
```

O instalador é gerado em `target\installer\Assinador-WhatsApp-x64.msi`.
