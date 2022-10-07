const web3 = require('@solana/web3.js')
const fetch = require('node-fetch')
const minimist = require('minimist')
const bs58 = require('bs58')
const fs = require('fs')
const { EventEmitter, once } = require('events')

const params = minimist(process.argv.slice(2))

const walletFromFile = (path) => web3.Keypair.fromSecretKey(new Uint8Array(JSON.parse(fs.readFileSync(path).toString())))

const AUTH_API_BASE_URL = 'https://auth.marinade.finance'
const SOLANA_CLUSTER_URL = 'https://api.mainnet-beta.solana.com'
const MTX_URL = 'https://rpc.mtx-perf-eu-central-1.marinade.finance'
// const MTX_URL = 'https://rpc.mtx-dev-eu-central-1.marinade.finance'
// const MTX_URL = 'http://localhost:3000'
const TX_COUNT = Number(params['tx-count']) || 0
const USER_WALLET_PATH = params['user-wallet'] || null
const USER_WALLET = USER_WALLET_PATH ? walletFromFile(USER_WALLET_PATH) : web3.Keypair.generate()
const TO_PUBKEY = params['to-pubkey'] || web3.Keypair.generate().publicKey

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

const buildDemoTx = (user, toPubkey, recentBlockhash) => new web3.Transaction({
  recentBlockhash
}).add(web3.SystemProgram.transfer({
  fromPubkey: user.publicKey,
  toPubkey: toPubkey,
  lamports: 1,
}))

const genDemoTxs = function * (user, toPubkey, recentBlockhash) {
  for (let i = 0; i < TX_COUNT; i++) {
    yield buildDemoTx(user, toPubkey, recentBlockhash)
  }
}

async function * genSignedDemoTxs (user, toPubkey, recentBlockhash) {
  let signaturePromiseBuff = []
  let txBuff = []
  const BUF_MAX = 10
  for (const tx of genDemoTxs(user, toPubkey, recentBlockhash)) {
    txBuff.push(tx)
    signaturePromiseBuff.push(tx.sign(user))
    if (txBuff.length == BUF_MAX) {
      await Promise.all(signaturePromiseBuff)
      yield * txBuff
      signaturePromiseBuff = []
      txBuff = []
    }
  }
  if (txBuff.length > 0) {
    await Promise.all(signaturePromiseBuff)
    yield * txBuff
  }
}

const Event = {
  TASK_FINISHED: 'TASK_FINISHED',
  TASK_REQUEST: 'TASK_REQUEST',
}

const run = async () => {
  const cluster = new web3.Connection(SOLANA_CLUSTER_URL)
  const user = USER_WALLET
  const toPubkey = TO_PUBKEY

  const authToken = await authenticate(user)
  console.log('TOKEN', authToken)

  const { blockhash: recentBlockhash } = await cluster.getRecentBlockhash()

  const MAX_PARALLEL_REQUESTS = 16
  let parallelRequests = 0
  let totalRequestsFinished = 0
  const timer = process.hrtime()
  const limitter = new EventEmitter()

  limitter.on(Event.TASK_REQUEST, async (task) => {
    parallelRequests++
    await task
    parallelRequests--
    limitter.emit(Event.TASK_FINISHED)
  })
  limitter.on(Event.TASK_FINISHED, () => {
    const [s, ns] = process.hrtime(timer)
    const duration = s + ns / 1e9
    totalRequestsFinished++
    console.log("Finished requests:", totalRequestsFinished, "Total time:", duration, "TPS:", totalRequestsFinished / duration)
  })

  for await (const signedTx of genSignedDemoTxs(user, toPubkey, recentBlockhash)) {
    if (parallelRequests == MAX_PARALLEL_REQUESTS) {
      await once(limitter, Event.TASK_FINISHED)
    }
    limitter.emit(Event.TASK_REQUEST, sendPriorityTransaction(authToken, signedTx))
  }
}

run()
