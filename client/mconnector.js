const fs = require('fs');
const web3 = require('@solana/web3.js')

const path = require("path");
const PROTO_PATH = path.join(__dirname, '..', 'proto', 'mtransaction.proto');

const protoLoader = require('@grpc/proto-loader');
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {keepCase: true});

const grpc = require('@grpc/grpc-js');
const validatorProto = grpc.loadPackageDefinition(packageDefinition).validator;

function connect() {
    const ssl_creds = grpc.credentials.createSsl(
        fs.readFileSync(path.join(__dirname, '..', 'certs', 'localhost.cert')),
    );
    const mtransactionClient = new validatorProto.MTransaction(`localhost:50051`, ssl_creds);
    const call = mtransactionClient.TxStream({message: 'Listening for transactions'}, (err, message) => {
        console.log(err, message);
    });
    call.on('data', ({ data }) => {
        const tx = web3.Transaction.populate(web3.Message.from(Buffer.from(data, 'base64')))
        console.log(data);
    });
    call.on('end',function(){
        console.log("Stream ended")
        connect();
    });
}

function main() {
    connect();
}

main();
