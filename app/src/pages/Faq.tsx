import { NATIVE, PROTOCOL_NAME, STABLE_SYMBOL } from "../config";
import { tr } from "../lib/i18n";
import { usePublicMode } from "../lib/AccountContext";
import { href } from "../router";

const QA = (pub: boolean): { q: string; a: string }[] => [
  {
    q: tr("Kann ich über die Seite einzahlen und prägen?", "Can I deposit and mint via the site?"),
    a: pub
      ? tr(
          `Ja, mit deiner Browser-Wallet (Kastle oder KasWare). Die Seite baut die Transaktion, deine Wallet zeigt sie dir und signiert sie; die Schlüssel verlassen die Wallet nie, K.Lend speichert keine. Vor dem Senden prüft der Server die signierte Transaktion noch einmal vollständig. Die Verträge sind experimentell und nicht professionell geprüft (nur KI-Audits). Setze nur Beträge ein, die du verlieren kannst.`,
          `Yes, with your browser wallet (Kastle or KasWare). The site builds the transaction, your wallet shows it to you and signs it; the keys never leave the wallet, K.Lend stores none. Before sending, the server checks the signed transaction once more in full. The contracts are experimental and not professionally audited (AI audits only). Only use amounts you can afford to lose.`,
        )
      : tr(
      `Ja, wenn die Seite lokal auf deinem Rechner läuft. Deine Browser-Wallet bleibt dabei nur lesend. Die Aktionen führt der lokale Server über ghostctl aus, und zwar mit einer Schlüsseldatei aus keys/ auf diesem Rechner. Vor jedem Senden steht eine Prüfung (Probelauf). Die Verträge sind experimentell und nicht professionell geprüft (nur ein KI-Audit, siehe AUDIT.md). Setze nur Beträge ein, die du verlieren kannst.`,
      `Yes, if the site runs locally on your machine. Your browser wallet stays read-only throughout. The actions are carried out by the local server via ghostctl, using a key file from keys/ on this machine. Every send is preceded by a check (dry run). The contracts are experimental and not professionally audited (only an AI audit, see AUDIT.md). Only use amounts you can afford to lose.`,
    ),
  },
  {
    q: tr(`Was ist ${STABLE_SYMBOL}?`, `What is ${STABLE_SYMBOL}?`),
    a: tr(
      `Ein Stablecoin, der 1 US-Dollar wert sein soll. Er entsteht, wenn jemand ${NATIVE} in einen Vault legt und dagegen ${STABLE_SYMBOL} prägt, und verschwindet wieder, wenn die Schuld getilgt wird. Der Name ist ein Arbeitstitel.`,
      `A stablecoin meant to be worth 1 US dollar. It's created when someone deposits ${NATIVE} into a vault and mints ${STABLE_SYMBOL} against it, and disappears again when the debt is repaid. The name is a working title.`,
    ),
  },
  {
    q: tr(`Was passiert, wenn der ${NATIVE}-Kurs fällt?`, `What happens if the ${NATIVE} price falls?`),
    a: tr(
      `Deine Quote sinkt. Für sie zählen Schuld und offener Zins zusammen. Unter 200 % kannst du nichts mehr prägen oder abheben. Je Vault lassen sich außerdem höchstens 50 ${STABLE_SYMBOL} prägen, die Zahl der Vaults ist nicht begrenzt. Unter 150 % darf jeder deinen Vault liquidieren, ganz oder teilweise: Er verbrennt einen Teil deiner Schuld oder die ganze und bekommt dafür ${NATIVE} im Wert des Betrags plus 10 %. Den Rest behältst du, ebenso die geprägten ${STABLE_SYMBOL}. Fällt die Deckung unter etwa 110 %, bekommt der Liquidator die ganze Sicherheit, der Vault endet, und die Restschuld wird ausgebucht. Diese ${STABLE_SYMBOL} sind dann nicht mehr gedeckt. Ob und wann jemand liquidiert, ist nicht garantiert.`,
      `Your ratio drops. Debt and open interest count together for it. Below 200 % you can no longer mint or withdraw. Each vault can also mint at most 50 ${STABLE_SYMBOL}; the number of vaults isn't limited. Below 150 % anyone may liquidate your vault, fully or partially: they burn part or all of your debt and receive ${NATIVE} worth that amount plus 10 % in return. You keep the rest, as well as the ${STABLE_SYMBOL} you minted. If coverage falls below about 110 %, the liquidator gets all the collateral, the vault ends, and the remaining debt is written off. Those ${STABLE_SYMBOL} are then no longer backed. Whether and when someone liquidates is not guaranteed.`,
    ),
  },
  {
    q: tr("Wie vermeide ich eine Liquidation?", "How do I avoid a liquidation?"),
    a: tr(
      `Halte Abstand zum Liquidationspreis. Du kannst jederzeit ${NATIVE} nachschießen oder einen Teil tilgen. Beides geht nur mit dem Schlüssel des Besitzers. Offener Zins senkt die Quote mit der Zeit, behalte ihn also im Blick. Einen Agenten, der das automatisch für dich erledigt, gibt es nicht.`,
      `Keep a margin to the liquidation price. You can add ${NATIVE} collateral or repay part of the debt at any time. Both only work with the owner's key. Open interest lowers the ratio over time, so keep an eye on it. There is no agent that does this for you automatically.`,
    ),
  },
  {
    q: tr("Wer legt den Zins fest?", "Who sets the interest rate?"),
    a: tr(
      `Eine feste Regel, ausgeführt vom GHOST-Agenten. Er misst in jeder Runde, was ${STABLE_SYMBOL} am Markt kostet (Kurs im Tauschpool mal ${NATIVE}-Preis), und nimmt den Median der Messungen der letzten Stunde. Liegt er unter 0,995 USD, steigt der Zins um 0,5 Prozentpunkte, über 1,005 USD sinkt er um 0,5 Prozentpunkte, immer zwischen 2 % (Grundzins) und 20 % pro Jahr und höchstens einmal pro Stunde; unter 2 % hebt der Agent ihn stündlich um 0,5 Punkte an. Obergrenze und Schrittweite erzwingt der Vertrag. Geändert wird erst ab 6 Messungen und nur, wenn der Tauschpool mindestens 10 ${STABLE_SYMBOL} hält. Ein höherer Zins macht Schulden teurer, Schuldner kaufen ${STABLE_SYMBOL} und tilgen, und der Kurs steigt; ein niedrigerer macht Prägen attraktiver. Gesetzt wird der Satz über das Orakel, dafür verlangt der Vertrag die Signaturen der Unterzeichner. Zum Start ist das allein der Betreiber.`,
      `A fixed rule, carried out by the GHOST agent. In every round it measures what ${STABLE_SYMBOL} costs on the market (pool price times the ${NATIVE} price) and takes the median of the last hour's measurements. Below 0.995 USD the rate rises by 0.5 percentage points, above 1.005 USD it falls by 0.5 percentage points, always between 2 % (base rate) and 20 % per year and at most once per hour; below 2 % the agent raises it by 0.5 points per hour. The contract enforces the cap and the step size. It only changes with at least 6 measurements and when the swap pool holds at least 10 ${STABLE_SYMBOL}. A higher rate makes debt more expensive, debtors buy ${STABLE_SYMBOL} and repay, and the price rises; a lower one makes minting more attractive. The rate is set through the oracle, which requires the signers' signatures. At launch that is the operator alone.`,
    ),
  },
  {
    q: tr("Wann und wie bezahle ich den Zins?", "When and how do I pay the interest?"),
    a: tr(
      `Beim Schließen. Der Zins wird getrennt von der Schuld in USD verbucht: Die Schuld ist genau die Menge ${STABLE_SYMBOL}, die du geprägt hast, und beim Tilgen verbrennst du nur diese. Für die Quoten zählen Schuld und Zins aber zusammen. Schließt du den schuldenfreien Vault, geht der Zins in ${NATIVE} zum Orakelpreis an die Zinsadresse (die Adresse des Betreibers), alle übrigen ${NATIVE} an dich. Liegt der Zins unter 0,2 ${NATIVE}, wird er erlassen. Solange der Zins offen ist, verzinst er sich mit. Die Seite zeigt vor dem Schließen, wie viel es ist. Zehrt der Zins die ganze Sicherheit auf, etwa nach einer Liquidation, darf jeder den Vault zugunsten der Zinsadresse auflösen („Auflösen (Zinsadresse)“ auf der Seite „Vault“).`,
      `When closing. Interest is booked separately from the debt, in USD: the debt is exactly the amount of ${STABLE_SYMBOL} you minted, and repaying only burns that. For the ratios, however, debt and interest count together. When you close the debt-free vault, the interest goes to the interest address (the operator's address) in ${NATIVE} at the oracle price, all remaining ${NATIVE} go to you. If the interest is below 0.2 ${NATIVE}, it is waived. As long as the interest is open, it bears interest too. The site shows the amount before closing. If the interest eats up all the collateral, for example after a liquidation, anyone may dissolve the vault in favor of the interest address (“Dissolve (interest address)” on the “Vault” page).`,
    ),
  },
  {
    q: tr("Was ist die Rücknahme zu 1 USD?", "What is redemption at 1 USD?"),
    a: tr(
      `Jeder kann ${STABLE_SYMBOL} an einem Vault zurückgeben, der mindestens bei 150 % steht, und bekommt dafür ${NATIVE} im Wert von 1 USD je ${STABLE_SYMBOL}, abzüglich 1 %. Die ${STABLE_SYMBOL} werden verbrannt und die Schuld des Vaults sinkt um denselben Betrag. Das 1 % bleibt beim Vault-Besitzer, und im Vault bleiben mindestens 0,2 ${NATIVE}. Zurückgegeben wird mindestens 1 ${STABLE_SYMBOL} oder die ganze Schuld des Vaults. Kostet ${STABLE_SYMBOL} am Markt deutlich weniger als 1 USD, lohnt es sich, zu kaufen und zurückzugeben – das hält den Kurs von unten. Als Vault-Besitzer heißt das: Ein Teil deiner Sicherheit kann jederzeit gegen Schuld getauscht werden, und zwar zum Orakelpreis. Springt ${NATIVE} um mehr als 20 % nach oben, übernimmt der Agent den neuen Preis erst nach drei bestätigenden Runden. In dieser Zeit bekommt ein Rücknehmer mehr, als der Markt hergibt, und die Differenz trägt der Vault-Besitzer.`,
      `Anyone can return ${STABLE_SYMBOL} to a vault that is at 150 % or more and receives ${NATIVE} worth 1 USD per ${STABLE_SYMBOL}, minus 1 %. The ${STABLE_SYMBOL} are burned and the vault's debt drops by the same amount. The 1 % stays with the vault owner, and at least 0.2 ${NATIVE} remain in the vault. At least 1 ${STABLE_SYMBOL} or the vault's whole debt is returned. If ${STABLE_SYMBOL} costs clearly less than 1 USD on the market, it pays to buy and redeem – that supports the price from below. As a vault owner this means: part of your collateral can be swapped against debt at any time, at the oracle price. If ${NATIVE} jumps up by more than 20 %, the agent only adopts the new price after three confirming rounds. During that time a redeemer receives more than the market would give, and the vault owner bears the difference.`,
    ),
  },
  {
    q: tr("Gibt es schon Keeper oder KI-Agenten?", "Are there already keepers or AI agents?"),
    a: tr(
      "Ein erster Agent ist gebaut: „GHOST-Agent starten.command“ bzw. `ghostctl agent`. Er hält beim Betreiber das Orakel frisch, passt den Zins an den GHOST-Kurs an und löst Vaults unter 150 % ab. Das tut er nur, wenn auch der aktuelle Marktpreis die Unterdeckung bestätigt, nie mit Verlust und nur mit eigenen GHOST. Jeder kann ihn als Liquidator betreiben, dafür braucht er keine Unterzeichner-Schlüssel. Kommt 2 Stunden lang kein Preis, friert er das Orakel ein. Ob einer läuft, ist nicht garantiert. Signierte Aufträge, die Agenten gebündelt ausführen, und eine MCP-Schnittstelle gibt es noch nicht.",
      "A first agent has been built: “GHOST-Agent starten.command” or `ghostctl agent`. It keeps the oracle fresh at the operator's end, adjusts the interest rate to the GHOST price and liquidates vaults below 150 %. It only does so if the current market price also confirms the shortfall, never at a loss, and only with its own GHOST. Anyone can run it as a liquidator; that requires no signer keys. If no price arrives for 2 hours, it freezes the oracle. Whether one is running is not guaranteed. Signed orders that agents execute in bundles, and an MCP interface, do not exist yet.",
    ),
  },
  {
    q: tr("Wer kontrolliert das Orakel?", "Who controls the oracle?"),
    a: tr(
      "Zum Start der Betreiber allein: Er ist der einzige Unterzeichner, 1 Signatur genügt. Im Unterzeichner-Register, einem eigenen Vertrag, steht nur der Hash des Unterzeichner-Satzes (höchstens 9 Schlüssel, echte Mehrheit 2t > n). Wer die nötigen Schlüssel hält, kann jeden Preis setzen und damit jeden Vault liquidierbar machen oder ungedeckte Prägung erlauben. Der Vertrag begrenzt Sprünge nur auf ×2 bzw. ÷2 je Update bei mindestens etwa 1 Minute Abstand; in wenigen Minuten ist damit jeder Preis zwischen 0,00001 und 900 USD erreichbar. Weitere unabhängige Unterzeichner lassen sich später ohne neues Deployment einsetzen. Jeder davon würde einen eigenen Agenten betreiben, der nur signiert, wenn sein eigener Preisabruf passt.",
      "At launch, the operator alone: they are the only signer, 1 signature suffices. The signer register, a separate contract, only stores the hash of the signer set (at most 9 keys, true majority 2t > n). Whoever holds the required keys can set any price and thereby make any vault liquidatable or allow uncovered minting. The contract only limits jumps to ×2 or ÷2 per update with at least about 1 minute between updates; within a few minutes any price between 0.00001 and 900 USD can be reached. Further independent signers can be added later without a new deployment. Each would run their own agent that only signs if its own price check matches.",
    ),
  },
  {
    q: tr("Wie werden die Unterzeichner ausgetauscht?", "How are the signers replaced?"),
    a: tr(
      "Ein neuer Satz wird öffentlich angekündigt (die Ankündigung steht in der Transaktion) und erst nach 14 Tagen Wartezeit aktiviert; aktivieren darf dann jeder. Die Wartezeit erzwingt der Konsens über eine relative Sperre. In dieser Zeit kann der aktuelle Satz die Ankündigung absagen, und wer dem neuen Satz nicht traut, kann seinen Vault schließen oder GHOST einlösen. Ein optionaler Notfallsatz darf erst nach 30 Tagen ohne Preis-Update einen Austausch ankündigen, danach gelten wieder 14 Tage; jedes Preis-Update macht diese Ankündigung ungültig. Zum Start gibt es keinen Notfallsatz.",
      "A new set is announced publicly (the announcement is in the transaction) and only activated after a 14-day waiting period; anyone may then activate it. The waiting period is enforced by consensus via a relative lock. During it the current set can cancel the announcement, and anyone who does not trust the new set can close their vault or redeem GHOST. An optional emergency set may only announce a replacement after 30 days without a price update, followed again by 14 days; every price update invalidates that announcement. At launch there is no emergency set.",
    ),
  },
  {
    q: tr("Was passiert, wenn kein Preis mehr kommt?", "What happens if no price arrives?"),
    a: tr(
      "Kommt 2 Stunden lang kein Preis, darf jeder das Orakel einfrieren; der GHOST-Agent tut das automatisch. Eingefroren sind Prägen, Rücknahme, Liquidieren, Auflösen, Abheben bei offener Schuld und der Tausch im Pool gesperrt. Einzahlen, Tilgen, Schließen, Abheben ohne Schuld sowie Liquidität einlegen und abziehen gehen weiter. Das nächste gültige Preis-Update taut alles wieder auf.",
      "If no price arrives for 2 hours, anyone may freeze the oracle; the GHOST agent does this automatically. While frozen, minting, redemption, liquidation, dissolving, withdrawing with open debt and swapping in the pool are blocked. Depositing, repaying, closing, withdrawing without debt and adding or removing liquidity still work. The next valid price update unfreezes everything.",
    ),
  },
  {
    q: tr("Warum braucht es keine Bridge?", "Why is no bridge needed?"),
    a: tr(
      `Alles läuft direkt auf der Basiskette von Kaspa. Deine ${NATIVE} werden nicht in ein anderes Netz verpackt. Damit entfällt das Risiko, dass eine Brücke gehackt wird.`,
      `Everything runs directly on Kaspa's base chain. Your ${NATIVE} is never wrapped into another network. That removes the risk of a bridge getting hacked.`,
    ),
  },
  {
    q: tr("Ist das geprüft (auditiert)?", "Has this been audited?"),
    a: tr(
      "Nein. Die Verträge sind lokal gegen die echte Skript-Engine getestet und haben einen vollständigen Durchlauf im Testnetz hinter sich (Prägen, Tilgen, Liquidation). Ein unabhängiges Audit steht aus. Bitte behandle alles hier als Experiment.",
      "No. The contracts are tested locally against the real script engine and have been through a full run on testnet (minting, repaying, liquidation). An independent audit is still pending. Please treat everything here as an experiment.",
    ),
  },
  {
    q: tr("Welche Wallet brauche ich?", "Which wallet do I need?"),
    a: pub
      ? tr(
          "Am Rechner Kastle oder KasWare als Browser-Erweiterung. Am Handy geht es nur im Browser einer Wallet-App: bei KasWare (Android-App) im eingebauten Browser; Kastle zeigt unter „Explore“ nur geprüfte Apps, dort ist K.Lend noch nicht aufgenommen. GHOST zeigen die gängigen Wallets noch nicht an; dafür ist die Seite „Wallet“ da.",
          "On a computer, Kastle or KasWare as a browser extension. On a phone it only works inside a wallet app's browser: KasWare (Android app) in its built-in browser; Kastle's “Explore” only lists verified apps and K.Lend is not listed yet. Common wallets don't display GHOST yet; that's what the “Wallet” page is for.",
        )
      : tr(
      "Zum Handeln keine: Aktionen laufen mit einer Schlüsseldatei in keys/, die du unter „Wallet“ anlegst und mit KAS aus deiner Wallet auflädst. KasWare oder Kastle kannst du zusätzlich verbinden. Die Seite liest davon nur Adresse, Netzwerk, Guthaben und öffentlichen Schlüssel und fordert nie eine Signatur an. GHOST zeigen die gängigen Wallets nicht an. Dafür ist die Seite „Wallet“ da: Guthaben, Senden und Empfangen von KAS und GHOST.",
      "None for acting: actions run with a key file in keys/, which you create under “Wallet” and fund with KAS from your wallet. You can additionally connect KasWare or Kastle. The site only reads their address, network, balance and public key, and never requests a signature. Common wallets don't display GHOST. That's what the “Wallet” page is for: balances, sending and receiving KAS and GHOST.",
    ),
  },
  {
    q: tr(`Wie empfange ich ${STABLE_SYMBOL}?`, `How do I receive ${STABLE_SYMBOL}?`),
    a: pub
      ? tr(
          `Unter „Wallet“ steht die Adresse deiner verbundenen Wallet, auch als QR-Code. An dieselbe Adresse gehen ${NATIVE} und ${STABLE_SYMBOL}. ${STABLE_SYMBOL}, die über K.Lend an dich gesendet werden, erscheinen dort sofort. Kam eine Sendung auf anderem Weg, trägst du unter „Wallet“ den Betrag ein, den dir der Absender nennt, und die Seite sucht genau diesen Token.`,
          `Under “Wallet” you'll find the address of your connected wallet, also as a QR code. ${NATIVE} and ${STABLE_SYMBOL} both go to that address. ${STABLE_SYMBOL} sent to you via K.Lend shows up there immediately. If a transfer came another way, enter the amount the sender gives you under “Wallet”, and the site looks for exactly that token.`,
        )
      : tr(
      `Unter „Wallet“ steht deine Adresse, auch als QR-Code. An dieselbe Adresse gehen ${NATIVE} und ${STABLE_SYMBOL}. ${NATIVE} erscheinen von selbst. ${STABLE_SYMBOL} liegen in eigenen Token-UTXOs, deren Adresse vom Betrag abhängt. Kommt eine Sendung von einem anderen Rechner, trägst du unter „Wallet“ den Betrag ein, den dir der Absender nennt, und die Seite sucht genau diesen Token. Was du von dieser Seite aus an eigene Schlüsseldateien schickst, erscheint sofort.`,
      `Under “Wallet” you'll find your address, also as a QR code. ${NATIVE} and ${STABLE_SYMBOL} both go to the same address. ${NATIVE} shows up automatically. ${STABLE_SYMBOL} sits in its own token UTXOs, whose address depends on the amount. If a transfer comes from another machine, you enter the amount the sender gives you under “Wallet”, and the site looks for exactly that token. Whatever you send to your own key files from this site appears immediately.`,
    ),
  },
  {
    q: tr("Beweist eine verschlüsselte Nachricht, wer sie geschrieben hat?", "Does an encrypted message prove who wrote it?"),
    a: tr(
      `Nein. Verschlüsselt wird an die öffentliche Adresse des Empfängers, und die kann jeder verwenden. Verschlüsselung sorgt nur dafür, dass niemand sonst mitlesen kann. Unter „Von“ zeigt „Eingegangene Nachrichten“ die Adressen, von denen die Zahlung kam, nicht den Verfasser des Textes. Bei einem Tresor ist die Nachricht im Vertrag fest gebunden: Sie wurde beim Anlegen hinterlegt, jede Zahlung trägt genau sie, und wer die Zahlung auslöst, kann sie weder weglassen noch ändern. Wer sie geschrieben hat, beweist auch das nicht: Den Besitzer eines Tresors kann beim Anlegen jeder frei eintragen, ohne dessen Signatur. Hast du den Tresor-Code übernommen, nennt die Seite den Besitzer als Angabe des Codes und warnt, wenn die Nachricht anders lautet als die Beschreibung im Code. Aufforderungen in Nachrichten, etwa an eine neue Adresse zu zahlen oder Geld zurückzuschicken, immer auf anderem Weg beim Absender nachprüfen.`,
      `No. Messages are encrypted to the recipient's public address, which anyone can use. Encryption only ensures that nobody else can read along. Under “From”, “Received messages” shows the addresses the payment came from, not the author of the text. With a vault, the message is fixed in the contract: it was stored when the vault was created, every payment carries exactly this message, and whoever triggers a payment can neither leave it out nor change it. That does not prove who wrote it either: anyone creating a vault can enter any owner without that owner's signature. If you added the vault code, the site names the owner as stated in the code and warns if the message differs from the description in the code. Always verify requests in messages, such as paying to a new address or sending money back, with the sender through another channel.`,
    ),
  },
  {
    q: tr("Kann ich die Nachricht eines Tresors später ändern?", "Can I change a vault's message later?"),
    a: tr(
      "Nein. Die Nachricht steht als Hash fest im Vertrag des Tresors; jede Zahlung muss genau diese Nachricht tragen, auch eine leere, wenn beim Anlegen keine angegeben wurde. Das gilt für jeden, der eine Zahlung auslöst, und auch für dich als Absender. Für eine neue Nachricht kündigst du den Tresor (der Rest kommt zu dir zurück) und legst einen neuen an; der Empfänger braucht dann den neuen Tresor-Code. Bei einer verschlüsselten Nachricht bindet der Vertrag die verschlüsselte Fassung; die lesbare Beschreibung im Tresor-Code prüft ghostctl beim Übernehmen mit dem Schlüssel des Empfängers. Fehlt er, zeigt die Tresor-Liste die Beschreibung als „laut Tresor-Code, nicht geprüft“.",
      "No. The message is fixed as a hash in the vault's contract; every payment must carry exactly this message, even an empty one if none was given when creating it. That applies to anyone who triggers a payment and to you as the sender. For a new message, cancel the vault (the remainder comes back to you) and create a new one; the recipient then needs the new vault code. For an encrypted message the contract binds the encrypted form; ghostctl checks the readable description in the vault code against it with the recipient's key when importing. Without that key, the vault list shows the description as “as stated in the vault code, not verified”.",
    ),
  },
  {
    q: tr("Wer löst die Zahlungen eines Tresors aus, und was kostet das?", "Who triggers a vault's payments, and what does it cost?"),
    a: tr(
      `Jeder darf, sobald ein Termin erreicht ist, meist der Empfänger oder ein GHOST-Agent. Der Vertrag lässt nur genau den Betrag an den Empfänger zu, nicht früher als zum Termin, und je Zahlung höchstens die Höchstgebühr (Standard 0,01 KAS, beim Anlegen mindestens 0,004 KAS: so viel kostet eine Zahlung mit der längsten Nachricht höchstens, mit Aufschlag) zusätzlich aus dem Tresor. Was davon nicht als Netzgebühr gebraucht wird, darf ein fremder Auslöser behalten; ghostctl und diese Seite nehmen nur die nötige Gebühr und lassen den Rest im Tresor. Nach Betrag und Höchstgebühr muss laut Vertrag nur etwas übrig bleiben. ghostctl und diese Seite zahlen nur, wenn danach mindestens 1 KAS im Tresor bleibt; reicht es dafür mit der Gebühr aus dem Tresor nicht mehr, zahlen sie nur noch mit Gebühr vom eigenen Schlüssel. Diese Seite fragt dann vorher nach, ghostctl im Mainnet ebenso. Ein laufender GHOST-Agent zahlt die Gebühr dagegen ohne Rückfrage vom Schlüssel seines Betreibers, sobald der Tresor sie nicht trägt, auch wenn die Netzgebühr über der Höchstgebühr liegt. Ein fremder Auslöser kann dagegen zahlen, solange Betrag und Höchstgebühr gedeckt sind, und weniger Rest lassen.`,
      `Anyone may, once a due date is reached, usually the recipient or a GHOST agent. The contract only allows exactly the amount to the recipient, not earlier than the due date, and at most the maximum fee (default 0.01 KAS, at least 0.004 KAS when creating: the most a payment with the longest message costs, plus a margin) per payment on top from the vault. Whatever is not needed as network fee may be kept by a third-party trigger; ghostctl and this site only take the fee needed and leave the rest in the vault. After amount and maximum fee, the contract only requires that something remains. ghostctl and this site only pay if at least 1 KAS stays in the vault afterwards; if that no longer works with the fee from the vault, they only pay with the fee from your own key. This site then asks first, and so does ghostctl on mainnet. A running GHOST agent, however, pays the fee without asking from its operator's key whenever the vault does not cover it, even if the network fee exceeds the maximum fee. A third-party trigger, however, can pay as long as amount and maximum fee are covered, leaving less remainder.`,
    ),
  },
  {
    q: tr("Wie funktioniert das Tauschen?", "How does swapping work?"),
    a: tr(
      `Der Tauschpool hält ${NATIVE} und ${STABLE_SYMBOL}. Der Kurs ergibt sich aus dem Verhältnis der beiden Reserven, wie bei Uniswap, und jeder Tausch zahlt 0,3 % Gebühr in den Pool. Getauscht wird nur, solange der Kurs bei 1 USD ± 3 % bleibt (gemessen am Orakelpreis). Vor dem Senden zeigt die Seite, was du bekommst, und legt einen Mindestbetrag 1 % darunter fest. Tauscht jemand anderes zuerst, baut ghostctl gegen den neuen Pool-Stand neu. Liegt das Ergebnis dann unter dem Mindestbetrag, bricht es ab und sendet nichts. Weniger als den Mindestbetrag bekommst du nie. Jeder kann Liquidität einlegen und bekommt dafür Anteile, die einen Teil der Gebühren tragen und sich jederzeit wieder abziehen lassen. Bei einem kleinen Pool verschiebt schon ein kleiner Tausch den Kurs spürbar.`,
      `The swap pool holds ${NATIVE} and ${STABLE_SYMBOL}. The price comes from the ratio of the two reserves, as with Uniswap, and every swap pays a 0.3 % fee into the pool. Swaps only go through while the price stays within 1 USD ± 3 % (measured against the oracle price). Before sending, the site shows what you'll receive and sets a minimum amount 1 % below that. If someone else swaps first, ghostctl rebuilds against the new pool state; if the result is then below the minimum, it aborts and sends nothing. You never receive less than the minimum amount. Anyone can supply liquidity and receives shares in return, which earn a portion of the fees and can be withdrawn again at any time. In a small pool, even a small swap moves the price noticeably.`,
    ),
  },
  {
    q: tr("Gehört die Seite zum Kaspa-Projekt?", "Is this site part of the Kaspa project?"),
    a: tr(
      `Nein. ${PROTOCOL_NAME} ist ein unabhängiges Community-Projekt und nicht mit Kaspa verbunden.`,
      `No. ${PROTOCOL_NAME} is an independent community project and not affiliated with Kaspa.`,
    ),
  },
  {
    q: tr("Ist das eine Anlageempfehlung?", "Is this investment advice?"),
    a: tr(
      "Nein. Experimentelles Open-Source-Projekt, nicht professionell geprüft, keine Anlageberatung. Kryptowerte können ihren Wert vollständig verlieren.",
      "No. Experimental open-source project, not professionally audited, not investment advice. Crypto assets can lose their entire value.",
    ),
  },
];

export function Faq() {
  const pub = usePublicMode();
  return (
    <div className="container section prose">
      <h1 tabIndex={-1} data-route-heading>
        {tr("Häufige Fragen", "Frequently asked questions")}
      </h1>
      <div className="faq">
        {QA(pub).map((x) => (
          <details key={x.q}>
            <summary>{x.q}</summary>
            <p>{x.a}</p>
          </details>
        ))}
      </div>
      <p className="muted">
        {tr("Mehr Hintergrund unter ", "More background at ")}
        <a href={href("so-funktioniert-es")}>{tr("So funktioniert es", "How it works")}</a>.
      </p>
    </div>
  );
}
