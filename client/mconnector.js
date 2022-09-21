const fs = require('fs');
const web3 = require('@solana/web3.js')

const path = require("path");
const PROTO_PATH = path.join(__dirname, '..', 'proto', 'mtransaction.proto');

const protoLoader = require('@grpc/proto-loader');
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {keepCase: true});

const grpc = require('@grpc/grpc-js');
const validatorProto = grpc.loadPackageDefinition(packageDefinition).validator;

process.on('uncaughtException', (err) => {
    console.log('Caught exception: ' + err + err.stack)
})

function restart(millisecondsToWait, r) {
    console.log(r, "Stream ended", millisecondsToWait);
    setTimeout(() => {
        connect();
    }, millisecondsToWait);
}

function connect(millisecondsToWait) {
    const r = (Math.random() + 1).toString(36).substring(7);
    const ssl_creds = grpc.credentials.createSsl(
        fs.readFileSync(path.join(__dirname, '..', 'certs', 'localhost.cert')),
    );
    const mtransactionClient = new validatorProto.MTransaction(`localhost:50051`, ssl_creds);
    const call = mtransactionClient.TxStream({message: 'Listening for transactions'}, (err, message) => {
        console.log(r, err, message);
    });
    call.on('data', ({ data }) => {
        const tx = web3.Transaction.populate(web3.Message.from(Buffer.from(data, 'base64')))
        console.log(r, data);
        millisecondsToWait = 500;
    });
    call.on('end', () => {
        millisecondsToWait += millisecondsToWait;
        mtransactionClient.close();
        console.log(r, "connection end")
        restart(millisecondsToWait, r);
    });
    call.on('error', (err) => {
        console.error(r, err);
    });
}

function main() {
    connect(500);
}

main();
