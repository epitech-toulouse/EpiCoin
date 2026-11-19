// The whole story, through the real interface, the real API, the real chain:
// account activation → receiving EPC → buying → delivery.

import { expect, test, type Browser, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const stack = JSON.parse(readFileSync(join(here, '.stack.json'), 'utf8')) as {
  adminActivationUrl: string;
  apiLog: string;
};

const PASSWORD = 'un mot de passe solide';

/**
 * The link of the latest email "sent" to `address`. In this stack the API uses its console
 * mailer, which writes every message to its log.
 */
async function emailedLink(address: string): Promise<string> {
  const marker = `to=${address}`;
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const log = readFileSync(stack.apiLog, 'utf8');
    const end = log.lastIndexOf(marker);
    if (end !== -1) {
      const links = log.slice(0, end).match(/https?:\/\/\S+#token=[0-9a-f]+/g);
      if (links?.length) return links[links.length - 1]!;
    }
    await new Promise((done) => setTimeout(done, 100));
  }
  throw new Error(`no email found for ${address}`);
}

async function newUserPage(browser: Browser): Promise<Page> {
  const context = await browser.newContext();
  return context.newPage();
}

async function activate(page: Page, link: string) {
  await page.goto(link);
  await page.getByLabel('Nouveau mot de passe').fill(PASSWORD);
  await page.getByLabel('Confirmez le mot de passe').fill(PASSWORD);
  await page.getByRole('button', { name: 'Activer mon compte' }).click();
}

async function login(page: Page, email: string) {
  await page.goto('/connexion');
  await page.getByLabel('Adresse email').fill(email);
  await page.getByLabel('Mot de passe').fill(PASSWORD);
  await page.getByRole('button', { name: 'Se connecter' }).click();
}

test.describe.configure({ mode: 'serial' });

let admin: Page;
let student: Page;
let staff: Page;
let manager: Page;

test('the first administrator activates their account', async ({ browser }) => {
  admin = await newUserPage(browser);
  await activate(admin, stack.adminActivationUrl);
  // Administrators land on their dashboard.
  await expect(admin.getByRole('heading', { name: 'Tableau de bord' })).toBeVisible();
  await expect(admin.getByText('Actions prioritaires')).toBeVisible();
  await admin.getByRole('link', { name: 'Utilisateurs' }).click();
  await expect(admin.getByRole('heading', { name: 'Utilisateurs' })).toBeVisible();
  await expect(admin.getByText('admin@epitech.eu')).toBeVisible();

  // The link is single-use.
  const again = await newUserPage(browser);
  await again.goto(stack.adminActivationUrl);
  await expect(again.getByRole('heading', { name: 'Lien invalide' })).toBeVisible();
  await again.context().close();
});

test('the administrator imports users, who receive an invitation', async () => {
  await admin.getByRole('button', { name: 'Importer un CSV' }).click();
  await admin
    .getByLabel('Ou collez son contenu')
    .fill(
      [
        'email;prenom;nom;cohorte;role',
        'ada.lovelace@epitech.eu;Ada;Lovelace;pge1;student',
        'grace.hopper@epitech.eu;Grace;Hopper;PGE 2;student',
        'alan.turing@epitech.eu;Alan;Turing;;staff',
        'linus.torvalds@epitech.eu;Linus;Torvalds;;shop_manager',
      ].join('\n'),
    );
  await admin.getByRole('button', { name: 'Importer', exact: true }).click();
  await expect(admin.getByText('4 compte(s) créé(s)')).toBeVisible();
  await admin.getByRole('button', { name: 'Terminé' }).click();

  await expect(admin.getByText('ada.lovelace@epitech.eu')).toBeVisible();
  await expect(admin.getByText('Invitation envoyée').first()).toBeVisible();
});

test('the administrator filters by cohort and moves a student with a CSV', async () => {
  const cohort = admin.getByLabel('Cohorte');
  await cohort.selectOption({ label: 'PGE 1 (1)' });
  await expect(admin.getByText('ada.lovelace@epitech.eu')).toBeVisible();
  await expect(admin.getByText('grace.hopper@epitech.eu')).toBeHidden();

  await admin.getByRole('button', { name: 'Importer un CSV' }).click();
  await admin
    .getByLabel('Ou collez son contenu')
    .fill('email;cohorte\ngrace.hopper@epitech.eu;pge1');
  await admin.getByRole('button', { name: 'Importer', exact: true }).click();
  await expect(admin.getByText(/1 changement\(s\) de cohorte/)).toBeVisible();
  await admin.getByRole('button', { name: 'Terminé' }).click();
  await expect(admin.getByText('grace.hopper@epitech.eu')).toBeVisible();
  await cohort.selectOption('');
});

test('a file with an invalid line is refused entirely', async () => {
  await admin.getByRole('button', { name: 'Importer un CSV' }).click();
  await admin
    .getByLabel('Ou collez son contenu')
    .fill('email;prenom;nom\nintrus@gmail.com;Un;Intrus\nok@epitech.eu;Tout;Bon');
  await admin.getByRole('button', { name: 'Importer', exact: true }).click();
  await expect(admin.getByText("Rien n'a été importé")).toBeVisible();
  await expect(admin.getByText(/Ligne 2/)).toBeVisible();
  await admin.getByRole('button', { name: 'Annuler' }).click();
  await admin.getByLabel('Rechercher').fill('ok@epitech.eu');
  await expect(admin.getByText('Aucun utilisateur ne correspond.')).toBeVisible();
  await admin.getByLabel('Rechercher').fill('');
});

test('invited users activate their accounts and land on their own space', async ({ browser }) => {
  student = await newUserPage(browser);
  await activate(student, await emailedLink('ada.lovelace@epitech.eu'));
  await expect(student.getByRole('heading', { name: 'Bonjour Ada' })).toBeVisible();
  await expect(student.getByTestId('balance')).toHaveText(/^0\s*EPC$/);

  staff = await newUserPage(browser);
  await activate(staff, await emailedLink('alan.turing@epitech.eu'));
  await expect(staff.getByRole('heading', { name: 'Distribuer des EPC' })).toBeVisible();

  manager = await newUserPage(browser);
  await activate(manager, await emailedLink('linus.torvalds@epitech.eu'));
  await expect(manager.getByRole('heading', { name: 'Commandes' })).toBeVisible();
});

test('each role only reaches its own screens', async () => {
  // A student who types an admin address is sent back home; the API would refuse anyway.
  await student.goto('/admin/utilisateurs');
  await expect(student.getByRole('heading', { name: 'Bonjour Ada' })).toBeVisible();
  await student.goto('/distribuer');
  await expect(student.getByRole('heading', { name: 'Bonjour Ada' })).toBeVisible();
  const refused = await student.request.get('/api/admin/users');
  expect(refused.status()).toBe(403);
});

test('the shop manager puts a product on sale', async () => {
  await manager.getByRole('link', { name: 'Produits' }).click();
  await manager.getByRole('button', { name: 'Nouveau produit' }).click();
  await manager.getByLabel('Nom').fill('Mug EpiCoin');
  await manager.getByLabel('Description').fill('Pour le café des lendemains de rush.');
  await manager.getByLabel('Prix (EPC)').fill('15');
  await manager.getByLabel('Stock').fill('3');
  await manager.getByRole('button', { name: 'Enregistrer' }).click();
  await expect(manager.getByRole('cell', { name: /Mug EpiCoin/ })).toBeVisible();
  await expect(manager.getByText('En vente')).toBeVisible();
});

test('a staff member rewards a student', async () => {
  // The on-chain authority follows the import within moments.
  await expect(async () => {
    await staff.reload();
    await expect(staff.getByText('Quota restant')).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 30_000 });
  await expect(staff.getByText(/^500\s*EPC$/).first()).toBeVisible();

  await staff.getByLabel('Rechercher').fill('lovelace');
  await staff.getByRole('checkbox', { name: /Ada Lovelace/ }).check();
  await expect(staff.getByTestId('selected-count')).toHaveText('1');
  await staff.getByLabel('Montant par étudiant (EPC)').fill('50');
  await staff.getByLabel('Motif').selectOption({ label: 'Entraide' });
  await staff.getByLabel('Commentaire (facultatif)').fill('Merci pour ton aide sur le projet');
  await staff.getByRole('button', { name: 'Distribuer', exact: true }).click();

  await expect(staff.getByText(/Distribution confirmée/)).toBeVisible();
  await expect(staff.getByText(/^450\s*EPC$/).first()).toBeVisible();

  await staff.getByRole('link', { name: 'Mes distributions' }).click();
  await expect(staff.getByRole('cell', { name: 'Ada Lovelace' })).toBeVisible();
  await expect(staff.getByText('Confirmée')).toBeVisible();
});

test('the student sees the EPC and why they got them', async () => {
  await student.reload();
  await expect(student.getByTestId('balance')).toHaveText(/^50\s*EPC$/);
  const line = student.getByRole('row', { name: /Entraide/ });
  await expect(line).toContainText('par Alan Turing');
  await expect(line).toContainText('Merci pour ton aide sur le projet');
  await expect(line).toContainText(/\+50\s*EPC/);
});

test('the student buys a product', async () => {
  await student.getByRole('link', { name: 'Boutique', exact: true }).click();
  await expect(student.getByRole('heading', { name: 'Mug EpiCoin' })).toBeVisible();
  await student.getByRole('button', { name: 'Ajouter Mug EpiCoin au panier' }).click();
  await student.getByRole('link', { name: /Voir mon panier/ }).click();

  await expect(student.getByTestId('cart-total')).toHaveText(/^15\s*EPC$/);
  // An impatient double click must not pay twice.
  await student.getByRole('button', { name: /^Payer/ }).dblclick();
  await expect(student.getByRole('heading', { name: 'Commande confirmée' })).toBeVisible();

  await student.getByRole('link', { name: 'Voir mes commandes' }).click();
  await expect(student.getByText('En préparation')).toBeVisible();
  await expect(student.getByText('1 × Mug EpiCoin')).toBeVisible();

  await student.getByRole('link', { name: 'Mon solde' }).click();
  await expect(student.getByTestId('balance')).toHaveText(/^35\s*EPC$/);
  await expect(student.getByRole('row', { name: /Achat en boutique/ })).toContainText(/−15\s*EPC/);
});

test('the shop manager prepares and hands over the order', async () => {
  await manager.getByRole('link', { name: 'Commandes' }).click();
  await expect(manager.getByText('Ada Lovelace')).toBeVisible();
  await expect(manager.getByText('1 × Mug EpiCoin')).toBeVisible();
  await manager.getByRole('button', { name: 'Marquer comme prête' }).click();
  await expect(manager.getByText('Aucune commande dans cette file.')).toBeVisible();

  await manager.getByRole('tab', { name: 'Prêtes à remettre' }).click();
  await manager.getByRole('button', { name: 'Valider la remise' }).click();
  await expect(manager.getByText('Aucune commande dans cette file.')).toBeVisible();

  await manager.getByRole('tab', { name: 'Remises' }).click();
  await expect(manager.getByText('Ada Lovelace')).toBeVisible();

  // The stock went down with the sale.
  await manager.getByRole('link', { name: 'Produits' }).click();
  await expect(manager.getByRole('row', { name: /Mug EpiCoin/ })).toContainText('2');
});

test('the student sees the order as delivered', async () => {
  await student.getByRole('link', { name: 'Mes commandes', exact: true }).click();
  await expect(student.getByText('Remise', { exact: true })).toBeVisible();
});

test('a cancelled order is refunded', async ({ browser }) => {
  // A second student gets EPC, buys, and the manager cancels.
  await staff.getByRole('link', { name: 'Distribuer', exact: true }).click();
  await staff.getByLabel('Rechercher').fill('hopper');
  await staff.getByRole('checkbox', { name: /Grace Hopper/ }).check();
  await staff.getByLabel('Montant par étudiant (EPC)').fill('20');
  await staff.getByRole('button', { name: 'Distribuer', exact: true }).click();
  await expect(staff.getByText(/Distribution confirmée/)).toBeVisible();

  const grace = await newUserPage(browser);
  await activate(grace, await emailedLink('grace.hopper@epitech.eu'));
  await expect(grace.getByTestId('balance')).toHaveText(/^20\s*EPC$/);
  await grace.getByRole('link', { name: 'Boutique', exact: true }).click();
  await grace.getByRole('button', { name: 'Ajouter Mug EpiCoin au panier' }).click();
  await grace.getByRole('link', { name: /Voir mon panier/ }).click();
  await grace.getByRole('button', { name: /^Payer/ }).click();
  await expect(grace.getByRole('heading', { name: 'Commande confirmée' })).toBeVisible();

  manager.once('dialog', (dialog) => void dialog.accept());
  await manager.getByRole('link', { name: 'Commandes' }).click();
  await expect(manager.getByText('Grace Hopper')).toBeVisible();
  await manager.getByRole('button', { name: 'Annuler et rembourser' }).click();
  await expect(manager.getByText('Aucune commande dans cette file.')).toBeVisible();

  await grace.getByRole('link', { name: 'Mes commandes', exact: true }).click();
  await expect(grace.getByText('Annulée et remboursée')).toBeVisible();
  await grace.getByRole('link', { name: 'Mon solde' }).click();
  await expect(grace.getByTestId('balance')).toHaveText(/^20\s*EPC$/);
  await expect(grace.getByRole('row', { name: /Remboursement/ })).toContainText(/\+15\s*EPC/);
  await grace.context().close();
});

test('the administrator follows everything: statistics, chain and audit', async () => {
  await admin.getByRole('link', { name: 'Tableau de bord' }).click();
  await expect(admin.getByText(/^70\s*EPC$/).first()).toBeVisible(); // 50 + 20 minted
  await expect(admin.getByRole('cell', { name: 'Alan Turing' })).toBeVisible();
  await expect(admin.getByRole('cell', { name: 'Entraide' })).toBeVisible();
  await expect(admin.getByText('1 commande(s), 1 remboursée(s)')).toBeVisible();

  await admin.getByRole('link', { name: 'Blockchain' }).click();
  await expect(admin.getByRole('heading', { name: 'Derniers blocs' })).toBeVisible();
  await expect(admin.getByText(/Achat en boutique de 15\s*EPC/).first()).toBeVisible();
  await expect(admin.getByText(/Remboursement de l'achat/)).toBeVisible();
  await expect(admin.getByRole('cell', { name: 'Trésorerie de la boutique' })).toBeVisible();
  // The explorer shows addresses; names only appear as the admin's own annotation.
  await admin.getByRole('table', { name: 'Derniers blocs' }).getByRole('link').first().click();
  await expect(admin.getByRole('heading', { name: /Bloc n° \d+/ })).toBeVisible();
  await expect(admin.getByText('Racine de l\'état')).toBeVisible();

  await admin.getByRole('link', { name: 'Audit' }).click();
  await expect(admin.getByRole('cell', { name: 'Import d’utilisateurs' }).first()).toBeVisible();
  await expect(admin.getByRole('cell', { name: 'Distribution d’EPC' }).first()).toBeVisible();
  await expect(admin.getByRole('cell', { name: 'Commande remboursée' })).toBeVisible();
});

test('the administrator distributes EPC too', async () => {
  await admin.getByRole('link', { name: 'Distribuer', exact: true }).click();
  await expect(admin.getByText('Quota restant')).toBeVisible();
  await admin.getByLabel('Rechercher').fill('hopper');
  await admin.getByRole('checkbox', { name: /Grace Hopper/ }).check();
  await admin.getByLabel('Montant par étudiant (EPC)').fill('10');
  await admin.getByLabel('Motif').selectOption({ label: 'Événement' });
  await admin.getByRole('button', { name: 'Distribuer', exact: true }).click();
  await expect(admin.getByText(/Distribution confirmée/)).toBeVisible();
  await expect(admin.getByText(/^490\s*EPC$/).first()).toBeVisible();

  await admin.getByRole('link', { name: 'Mes distributions' }).click();
  await expect(admin.getByRole('cell', { name: 'Grace Hopper' })).toBeVisible();
});

test('logging out really ends the session', async () => {
  await student.getByRole('button', { name: 'Se déconnecter' }).click();
  await expect(student.getByRole('heading', { name: 'Connexion' })).toBeVisible();
  await student.goto('/');
  await expect(student.getByRole('heading', { name: 'Connexion' })).toBeVisible();

  // A wrong password says so without saying more.
  await student.getByLabel('Adresse email').fill('ada.lovelace@epitech.eu');
  await student.getByLabel('Mot de passe').fill('pas le bon mot de passe');
  await student.getByRole('button', { name: 'Se connecter' }).click();
  await expect(student.getByRole('alert')).toHaveText('Email ou mot de passe incorrect.');

  await login(student, 'ada.lovelace@epitech.eu');
  await expect(student.getByTestId('balance')).toHaveText(/^35\s*EPC$/);
});

test('a session ended elsewhere sends the user back to the login screen', async () => {
  // Two tabs of the same browser share the session; logging out in one ends it for both.
  const otherTab = await student.context().newPage();
  await otherTab.goto('/boutique');
  await expect(otherTab.getByRole('heading', { name: 'Boutique' })).toBeVisible();
  await student.getByRole('button', { name: 'Se déconnecter' }).click();
  await expect(student.getByRole('heading', { name: 'Connexion' })).toBeVisible();

  // The other tab finds out at its next request, without a reload.
  await otherTab.getByRole('link', { name: 'Mon solde' }).click();
  await expect(otherTab.getByRole('heading', { name: 'Connexion' })).toBeVisible();
  await otherTab.close();
  await login(student, 'ada.lovelace@epitech.eu');
  await expect(student.getByTestId('balance')).toHaveText(/^35\s*EPC$/);
});

test('a staff member posts a challenge and a student wins it', async () => {
  await staff.getByRole('link', { name: 'Défis' }).click();
  await staff.getByRole('button', { name: 'Nouveau défi' }).click();
  await staff.getByLabel('Titre').fill('Énigme du campus');
  await staff.getByLabel('Description').fill('Quel langage fait tourner EpiCoin ?');
  await staff.getByLabel('Gain par étudiant (EPC)').fill('5');
  await staff.getByLabel('Réponse attendue').fill('Rust');
  await staff.getByRole('button', { name: 'Créer le défi' }).click();
  await expect(staff.getByRole('cell', { name: /^Énigme du campus/ })).toBeVisible();

  await student.getByRole('link', { name: 'Défis' }).click();
  const card = student.getByRole('article', { name: 'Énigme du campus' });
  await card.getByLabel('Votre réponse').fill('python');
  await card.getByRole('button', { name: 'Valider' }).click();
  await expect(card.getByText("Ce n'est pas la bonne réponse.")).toBeVisible();
  await card.getByLabel('Votre réponse').fill('  RUST ');
  await card.getByRole('button', { name: 'Valider' }).click();
  await expect(card.getByText(/Défi réussi : 5\s*EPC gagnés/)).toBeVisible();

  await student.getByRole('link', { name: 'Mon solde' }).click();
  await expect(student.getByTestId('balance')).toHaveText(/^40\s*EPC$/);
  await expect(student.getByText('Défi : Énigme du campus')).toBeVisible();
});

test('the interface is usable on a phone', async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 375, height: 700 } });
  const phone = await context.newPage();
  await login(phone, 'ada.lovelace@epitech.eu');
  await expect(phone.getByTestId('balance')).toBeVisible();

  // Navigation folds into a menu; nothing overflows sideways.
  await expect(phone.getByRole('link', { name: 'Boutique', exact: true })).toBeHidden();
  await phone.getByRole('button', { name: 'Menu' }).click();
  await phone.getByRole('link', { name: 'Boutique', exact: true }).click();
  await expect(phone.getByRole('heading', { name: 'Mug EpiCoin' })).toBeVisible();
  const overflow = () =>
    phone.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
  expect(await overflow()).toBeLessThanOrEqual(0);

  // Wide tables scroll inside their frame: no page, whoever the user, scrolls sideways.
  const pages: [string, string[]][] = [
    ['ada.lovelace@epitech.eu', ['/', '/panier', '/commandes']],
    ['admin@epitech.eu', ['/admin', '/admin/utilisateurs', '/admin/explorateur', '/admin/audit', '/gestion/produits', '/gestion/commandes']],
    ['alan.turing@epitech.eu', ['/distribuer', '/distributions']],
  ];
  for (const [email, routes] of pages) {
    await context.clearCookies();
    await login(phone, email);
    await expect(phone.getByRole('button', { name: 'Menu' })).toBeVisible();
    for (const route of routes) {
      await phone.goto(route);
      await expect(phone.getByRole('heading', { level: 1 })).toBeVisible();
      await phone.waitForLoadState('networkidle');
      expect(await overflow(), `${route} scrolls sideways`).toBeLessThanOrEqual(0);
    }
  }
  await context.close();
});

test('no secret ever reaches the browser', async () => {
  // Nothing the page stores is a key, and the session cookie is out of script reach.
  const stored = await student.evaluate(() =>
    JSON.stringify([{ ...localStorage }, { ...sessionStorage }, document.cookie]),
  );
  expect(stored).not.toContain('epicoin_session');
  expect(stored).not.toMatch(/[0-9a-f]{64}/);
  const cookies = await student.context().cookies();
  const session = cookies.find((cookie) => cookie.name === 'epicoin_session');
  expect(session?.httpOnly).toBe(true);
  expect(session?.sameSite).toBe('Lax');
});
