const web3 = require("@solana/web3.js");
const fetch = require("node-fetch");
const bs58 = require('bs58');

const fetchTxChallenge = async (pubKey) => {
  const txChallenge = await fetch(
    `https://auth.marinade.finance/auth/tx-challenge?pubkey=${pubKey}`,
    {
      method: "GET",
      headers: {
        "Content-Type": "application/json",
      },
    }
  );

  return txChallenge.json();
};

const verifyTxChallenge = async (
  tx_challenge_verifier,
  tx_signature
) => {
  const body = `${encodeURIComponent(
    "tx_challenge_verifier"
  )}=${encodeURIComponent(tx_challenge_verifier)}&${encodeURIComponent(
    "tx_signature"
  )}=${encodeURIComponent(tx_signature)}`;


  const verifiedTxChallenge = await fetch(
    `https://auth.marinade.finance/auth/tx-challenge`,
    {
      method: "post",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body,
    }
  );

  return verifiedTxChallenge.json();
};

(async () => {
// To be added later  
//   const connection = new web3.Connection(
//     web3.clusterApiUrl('devnet'),
//     {
//       httpHeaders: { Authorization: `Bearer ${key}` },
//     },
//     "confirmed"
//   );

  const from = web3.Keypair.generate();

  for (const i in [1, 2, 3]) {
    const res = await fetchTxChallenge(from.publicKey);
    const mtx = web3.Transaction.populate(
      web3.Message.from(Buffer.from(res.tx_msg_b64, "base64"))
    );
    
    await mtx.sign(from);

    const { signature } = mtx;

    const token = await verifyTxChallenge(
      res.tx_challenge_verifier,
      bs58.encode(signature)
    );

    console.log("TOKEN", token);
  }
})();
