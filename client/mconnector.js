const fs = require('fs');
const web3 = require('@solana/web3.js')

const path = require("path");
const PROTO_PATH = path.join(__dirname, '..', 'proto', 'mtransaction.proto');

const protoLoader = require('@grpc/proto-loader');
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {keepCase: true});

const grpc = require('@grpc/grpc-js');
const validatorProto = grpc.loadPackageDefinition(packageDefinition).validator;

function restart(millisecondsToWait, r, typ) {
    console.log(r, "Stream ended", millisecondsToWait, typ);
    setTimeout(() => {
        connect();
    }, millisecondsToWait);
}

function connect() {
    let millisecondsToWait = 500;
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
    });
    call.on('end', () => {
        millisecondsToWait += millisecondsToWait;
        mtransactionClient.close();
        console.log(r, "connection end")
        restart(millisecondsToWait);
    });
    call.on('error', (err) => {
        console.error(r, err);
    });
}

function main() {
    connect();
}

main();
