const web3 = require('@solana/web3.js')
const fetch = require('node-fetch')
const bs58 = require('bs58')

const AUTH_API_BASE_URL = 'https://auth.marinade.finance'
const SOLANA_CLUSTER_URL = 'https://api.devnet.solana.com'
const MTX_URL = 'http://localhost:3000'

const fetchTxChallenge = async (pubKey) => {
  const txChallenge = await fetch(`${AUTH_API_BASE_URL}/auth/tx-challenge?pubkey=${pubKey}`, {
    method: 'GET',
    headers: {
      'Content-Type': 'application/json',
    },
  })

  return txChallenge.json()
}

const verifyTxChallenge = async (
  tx_challenge_verifier,
  tx_signature
) => {
  const body = `${encodeURIComponent(
    'tx_challenge_verifier'
  )}=${encodeURIComponent(tx_challenge_verifier)}&${encodeURIComponent(
    'tx_signature'
  )}=${encodeURIComponent(tx_signature)}`


  const verifiedTxChallenge = await fetch(`${AUTH_API_BASE_URL}/auth/tx-challenge`, {
    method: 'post',
    headers: {
      'Content-Type': 'application/x-www-form-urlencoded',
    },
    body,
  })

  return verifiedTxChallenge.json()
}

const authenticate = async (user) => {
  const txChallenge = await fetchTxChallenge(user.publicKey)
  const authTx = web3.Transaction.populate(
    web3.Message.from(Buffer.from(txChallenge.tx_msg_b64, 'base64'))
  )
  await authTx.sign(user)
  const { signature } = authTx

  const { access_token } = await verifyTxChallenge(
    txChallenge.tx_challenge_verifier,
    bs58.encode(signature)
  )

  return access_token
}

let priorityTransactionCounter = 0
const sendPriorityTransaction = async (
  jwt,
  tx,
) => {
  const result = await fetch(`${MTX_URL}`, {
    method: 'post',
    body: JSON.stringify({
      jsonrpc: '2.0',
      method: 'sendPriorityTransaction',
      id: ++priorityTransactionCounter,
      params: [Buffer.from(tx.serialize()).toString('base64')],
    }),
    headers: { 'Content-Type': 'application/json' }
  })

  return result.json()
}

const buildDemoTx = async (user, recentBlockhash) => {
  const to = web3.Keypair.generate()
  return new web3.Transaction({
    recentBlockhash
  }).add(web3.SystemProgram.transfer({
    fromPubkey: user.publicKey,
    toPubkey: to.publicKey,
    lamports: 1,
  }))
}

(async () => {
  const cluster = new web3.Connection(SOLANA_CLUSTER_URL)
  const user = web3.Keypair.generate()

  // for (const i in [1, 2, 3]) {
  const authToken = await authenticate(user)
  console.log('TOKEN', authToken)

  const { blockhash } = await cluster.getRecentBlockhash()
  const tx = await buildDemoTx(user, blockhash)
  await tx.sign(user)
  console.log(await sendPriorityTransaction(authToken, tx))
  // }
})()
