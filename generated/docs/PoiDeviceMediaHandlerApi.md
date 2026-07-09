# \PoiDeviceMediaHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_poi_device_media_handler**](PoiDeviceMediaHandlerApi.md#create_poi_device_media_handler) | **POST** /api/v1/poi-device-medias | 
[**delete_poi_device_media_handler**](PoiDeviceMediaHandlerApi.md#delete_poi_device_media_handler) | **DELETE** /api/v1/poi-device-medias/{id} | 
[**get_poi_device_media_handler**](PoiDeviceMediaHandlerApi.md#get_poi_device_media_handler) | **GET** /api/v1/poi-device-medias/{id} | 
[**list_poi_device_media_handler**](PoiDeviceMediaHandlerApi.md#list_poi_device_media_handler) | **GET** /api/v1/poi-device-medias | 



## create_poi_device_media_handler

> models::PoiDeviceMediaExtendedModel create_poi_device_media_handler(create_poi_device_media_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_poi_device_media_schema** | [**CreatePoiDeviceMediaSchema**](CreatePoiDeviceMediaSchema.md) | Create PoI device media | [required] |

### Return type

[**models::PoiDeviceMediaExtendedModel**](PoiDeviceMediaExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_poi_device_media_handler

> delete_poi_device_media_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | id for deleting PoI device media | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_poi_device_media_handler

> models::PoiDeviceMediaExtendedModel get_poi_device_media_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | id for fetching PoI device media | [required] |

### Return type

[**models::PoiDeviceMediaExtendedModel**](PoiDeviceMediaExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_poi_device_media_handler

> models::PagedResponsePoiDeviceMediaExtendedModel list_poi_device_media_handler(page, limit, order_by, desc, poi_device_id, poi_id, poi_name, device_id, device_name, storage_secret_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**poi_device_id** | Option<**uuid::Uuid**> | poi device id |  |
**poi_id** | Option<**uuid::Uuid**> | PoI id |  |
**poi_name** | Option<**String**> | PoI name |  |
**device_id** | Option<**uuid::Uuid**> | device id |  |
**device_name** | Option<**String**> | device name |  |
**storage_secret_id** | Option<**uuid::Uuid**> | storage secret id |  |

### Return type

[**models::PagedResponsePoiDeviceMediaExtendedModel**](PagedResponse_PoiDeviceMediaExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

