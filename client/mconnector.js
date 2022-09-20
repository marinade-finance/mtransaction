const fs = require('fs');

const path = require("path");
const PROTO_PATH = path.join(__dirname, '..', 'proto', 'mtransaction.proto');

const protoLoader = require('@grpc/proto-loader');
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {keepCase: true});

const grpc = require('@grpc/grpc-js');
const validatorProto = grpc.loadPackageDefinition(packageDefinition).validator;

function connect() {
    const ssl_creds = grpc.credentials.createSsl(
        fs.readFileSync(path.join(__dirname, '..', 'cert', 'mtx-dev-eu-central-1.marinade.finance.cert')), 
        fs.readFileSync(path.join(__dirname, '..', 'cert', 'client-key.cer')), 
        fs.readFileSync(path.join(__dirname, '..', 'cert', 'client-cert.cer')),
    );
    const mtransactionClient = new validatorProto.MTransaction(`mtx-dev-eu-central-1.marinade.finance:50051`, ssl_creds);
    const call = mtransactionClient.EchoStream({message: 'Listening for transactions'}, (err, message) => {
        console.log(err, message);
    });
    call.on('data', (response) => {
        console.log(response);
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