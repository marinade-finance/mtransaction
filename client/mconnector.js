// const fs = require('fs');

const path = require("path");
const PROTO_PATH = path.join(__dirname, '..', 'proto', 'mtransaction.proto');

const protoLoader = require('@grpc/proto-loader');
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {keepCase: true});

const grpc = require('@grpc/grpc-js');
const validatorProto = grpc.loadPackageDefinition(packageDefinition).validator;

function connect() {
    const mtransactionClient = new validatorProto.MTransaction('192.168.200.176:50051', grpc.credentials.createInsecure());
    const call = mtransactionClient.EchoStream({message: 'Listening for transactions'});
    call.on('data', (response) => {
        console.log(response);
    });
    call.on('end',function(){
        console.log("Stream ended")
        connect();
    });
}

function main() {
    // const ssl_creds = grpc.credentials.createSsl(
    //     fs.readFileSync(path.join(__dirname, '..', 'cert', 'ca-cert.cer')),
    //     fs.readFileSync(path.join(__dirname, '..', 'cert', 'client-key.cer')),
    //     fs.readFileSync(path.join(__dirname, '..', 'cert', 'client-cert.cer')),
    // );
    // var client = new validator_proto.MTransaction('192.168.200.176:50051', ssl_creds);
    connect();
}

main();